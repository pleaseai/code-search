//! napi-rs native bindings — the in-process JS SDK for csp (`@pleaseai/csp-sdk`).
//!
//! This is the **library** distribution channel: it binds the `crates/csp` core
//! directly so JS callers run hybrid code search in-process (no subprocess, no
//! JSON round-trip), returning native objects. It is separate from the CLI/MCP
//! launcher under `npm/`, which execs the standalone `csp` binary.
//!
//! The public surface mirrors the README library contract: a `CspIndex` class
//! with `fromPath` / `fromGit` / `loadFromDisk` factories and `search` /
//! `findRelated` / `save` / `stats` methods, over camelCase `Chunk` /
//! `SearchResult` shapes. napi-rs converts Rust `snake_case` identifiers to JS
//! `camelCase` automatically (so `file_path` → `filePath`, `from_path` →
//! `fromPath`).
//!
//! NOTE: these bindings are synchronous; `fromPath` / `fromGit` do heavy work
//! (file walking, embedding, and — for git — a network clone) and will block the
//! Node event loop. Moving the build factories onto `AsyncTask` is a tracked
//! follow-up (see the crate README).

use std::collections::HashMap;
use std::path::Path;

use napi::bindgen_prelude::*;
use napi_derive::napi;

use csp::indexing::index::{
    CspIndex as CoreIndex, LoadOptions as CoreLoadOptions, QueryOptions as CoreQueryOptions,
};
use csp::search::SearchResult as CoreSearchResult;
use csp::types::{chunk_location, Chunk as CoreChunk, ContentType as CoreContentType};

/// Content type for indexing / search-pipeline selection. Mirrors the README
/// `ContentType` enum (`Code | Docs | Config`).
#[napi]
pub enum ContentType {
    Code,
    Docs,
    Config,
}

/// Build/load options shared by `fromPath` / `fromGit`.
#[napi(object)]
pub struct LoadOptions {
    /// Path to a Model2Vec model directory; omit to use the bundled default.
    pub model_path: Option<String>,
    /// Content types to index; omit for the default set.
    pub content: Option<Vec<ContentType>>,
}

/// Query options for `search` / `findRelated`.
#[napi(object)]
pub struct QueryOptions {
    /// Maximum number of results to return.
    pub top_k: Option<u32>,
    /// Restrict results to these languages.
    pub filter_languages: Option<Vec<String>>,
    /// Restrict results to chunks whose path matches one of these substrings.
    pub filter_paths: Option<Vec<String>>,
}

/// A single indexable unit of code (camelCase JS shape, with derived `location`).
#[napi(object)]
pub struct Chunk {
    pub content: String,
    pub file_path: String,
    pub start_line: u32,
    pub end_line: u32,
    pub language: Option<String>,
    /// `filePath:startLine-endLine`.
    pub location: String,
}

/// A scored search result.
#[napi(object)]
pub struct SearchResult {
    pub chunk: Chunk,
    pub score: f64,
}

/// Aggregate index statistics.
#[napi(object)]
pub struct IndexStats {
    pub indexed_files: u32,
    pub total_chunks: u32,
    /// language → chunk count.
    pub languages: HashMap<String, u32>,
}

/// Hybrid (dense + BM25) code-search index.
#[napi(js_name = "CspIndex")]
pub struct CspIndex {
    inner: CoreIndex,
}

#[napi]
impl CspIndex {
    /// Build an index from a local directory.
    #[napi(factory)]
    pub fn from_path(path: String, options: Option<LoadOptions>) -> Result<Self> {
        CoreIndex::from_path(Path::new(&path), &to_core_load_options(options))
            .map(|inner| Self { inner })
            .map_err(to_napi_err)
    }

    /// Build an index from a remote git URL (shallow clone into a temp dir).
    #[napi(factory)]
    pub fn from_git(
        url: String,
        options: Option<LoadOptions>,
        git_ref: Option<String>,
    ) -> Result<Self> {
        CoreIndex::from_git(&url, &to_core_load_options(options), git_ref.as_deref())
            .map(|inner| Self { inner })
            .map_err(to_napi_err)
    }

    /// Load an index previously persisted with `save`.
    #[napi(factory)]
    pub fn load_from_disk(dir: String) -> Result<Self> {
        CoreIndex::load_from_disk(Path::new(&dir))
            .map(|inner| Self { inner })
            .map_err(to_napi_err)
    }

    /// Hybrid search over the indexed chunks.
    #[napi]
    pub fn search(&self, query: String, options: Option<QueryOptions>) -> Vec<SearchResult> {
        self.inner
            .search(&query, &to_core_query_options(options))
            .iter()
            .map(to_js_result)
            .collect()
    }

    /// Find chunks similar to a seed chunk, excluding the seed itself.
    #[napi]
    pub fn find_related(&self, seed: Chunk, options: Option<QueryOptions>) -> Vec<SearchResult> {
        self.inner
            .find_related(&to_core_chunk(seed), &to_core_query_options(options))
            .iter()
            .map(to_js_result)
            .collect()
    }

    /// Persist the index to a directory.
    #[napi]
    pub fn save(&self, dir: String, content_hash: Option<String>) -> Result<()> {
        self.inner
            .save(Path::new(&dir), content_hash.as_deref())
            .map_err(to_napi_err)
    }

    /// Aggregate index statistics.
    #[napi]
    pub fn stats(&self) -> IndexStats {
        let s = self.inner.stats();
        IndexStats {
            indexed_files: s.indexed_files as u32,
            total_chunks: s.total_chunks as u32,
            languages: s
                .languages
                .into_iter()
                .map(|(lang, count)| (lang, count as u32))
                .collect(),
        }
    }
}

// --- conversions between the JS-facing and core types ---

fn to_napi_err(message: String) -> Error {
    Error::from_reason(message)
}

fn to_core_content(content: &ContentType) -> CoreContentType {
    match content {
        ContentType::Code => CoreContentType::Code,
        ContentType::Docs => CoreContentType::Docs,
        ContentType::Config => CoreContentType::Config,
    }
}

fn to_core_load_options(options: Option<LoadOptions>) -> CoreLoadOptions {
    match options {
        None => CoreLoadOptions::default(),
        Some(o) => CoreLoadOptions {
            model_path: o.model_path,
            content: o
                .content
                .map(|types| types.iter().map(to_core_content).collect()),
        },
    }
}

fn to_core_query_options(options: Option<QueryOptions>) -> CoreQueryOptions {
    match options {
        None => CoreQueryOptions::default(),
        Some(o) => CoreQueryOptions {
            top_k: o.top_k.map(|n| n as usize),
            filter_languages: o.filter_languages,
            filter_paths: o.filter_paths,
        },
    }
}

fn to_core_chunk(chunk: Chunk) -> CoreChunk {
    // `location` is derived; never trusted on the way in.
    CoreChunk {
        content: chunk.content,
        file_path: chunk.file_path,
        start_line: chunk.start_line,
        end_line: chunk.end_line,
        language: chunk.language,
    }
}

fn to_js_chunk(chunk: &CoreChunk) -> Chunk {
    Chunk {
        content: chunk.content.clone(),
        file_path: chunk.file_path.clone(),
        start_line: chunk.start_line,
        end_line: chunk.end_line,
        language: chunk.language.clone(),
        location: chunk_location(chunk),
    }
}

fn to_js_result(result: &CoreSearchResult) -> SearchResult {
    SearchResult {
        chunk: to_js_chunk(&result.chunk),
        score: result.score,
    }
}
