//! Index orchestration. Port of semble `index/create.py` (incremental reuse
//! from upstream #225).
//!
//! Walks files matching the resolved extensions, chunks them, enriches +
//! tokenizes text for BM25, embeds the chunks, and returns the populated
//! sparse/dense indexes alongside the chunk list and a per-file manifest.
//! When a [`PreviousIndex`] is supplied, files whose content hash is unchanged
//! reuse their previous chunks, vector rows, and BM25 postings; only changed
//! files are re-chunked and re-embedded, and deleted files' postings are
//! dropped.

use std::path::{Path, PathBuf};

use crate::chunking::source::chunk_source;
use crate::indexing::cache::sha256_hex;
use crate::indexing::dense::{embed_chunk_refs, Model, SelectableBasicBackend};
use crate::indexing::file_walker::walk_files;
use crate::indexing::files::{
    detect_language, get_extensions, get_max_file_bytes, DEFAULT_MAX_FILE_BYTES, MAX_FILE_BYTES_ENV,
};
use crate::indexing::sparse::{enrich_for_bm25, Bm25Index};
use crate::indexing::types::{make_chunk_id, FileManifest, FileManifestEntry, PreviousIndex};
use crate::tokens::tokenize;
use crate::types::{Chunk, ContentType};

/// 1 MB max file size to read and index.
#[deprecated(
    since = "0.1.10",
    note = "use `indexing::files::DEFAULT_MAX_FILE_BYTES` or `get_max_file_bytes()` \
            (the limit is now overridable via `CSP_MAX_FILE_BYTES`)"
)]
pub const MAX_FILE_BYTES: u64 = DEFAULT_MAX_FILE_BYTES;

/// Options for [`create_index_from_path`].
pub struct CreateIndexOptions<'a> {
    pub model: &'a Model,
    /// Extra extensions appended to those resolved from `content`.
    pub extensions: Option<Vec<String>>,
    /// Content selection (defaults to code-only, matching semble `_DEFAULT_CONTENT`).
    pub content: Option<Vec<ContentType>>,
    /// When set, chunk file paths are stored relative to this root.
    pub display_root: Option<PathBuf>,
    /// Max file size (bytes) to read and index; larger files are skipped with a
    /// warning. `None` resolves `CSP_MAX_FILE_BYTES` (default 1 MB) via
    /// [`get_max_file_bytes`].
    pub max_file_bytes: Option<u64>,
}

impl<'a> CreateIndexOptions<'a> {
    /// Options for `model` with every other field at its default: code-only
    /// content, no extra extensions, chunk paths as walked, and the size limit
    /// resolved from `CSP_MAX_FILE_BYTES`. Prefer
    /// `CreateIndexOptions { extensions: .., ..CreateIndexOptions::new(model) }`
    /// over a bare struct literal so a future option field does not break
    /// callers.
    pub fn new(model: &'a Model) -> Self {
        Self {
            model,
            extensions: None,
            content: None,
            display_root: None,
            max_file_bytes: None,
        }
    }
}

/// Result of [`create_index_from_path`].
#[derive(Debug)]
pub struct CreateIndexResult {
    pub bm25_index: Bm25Index,
    pub semantic_index: SelectableBasicBackend,
    pub chunks: Vec<Chunk>,
    /// Per-file content hash + chunk range, for the next incremental reindex.
    pub files: FileManifest,
}

/// A repository-controlled path rendered for a stderr diagnostic, with control
/// characters (ANSI escapes, newlines) escaped so a hostile file name in a
/// cloned repo cannot rewrite the terminal or forge diagnostic lines.
fn escape_control(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if c.is_control() {
            out.extend(c.escape_default());
        } else {
            out.push(c);
        }
    }
    out
}

/// The warning for files skipped for exceeding `max_file_bytes` — the count
/// plus the first five paths — or `None` when nothing was skipped. Port of
/// upstream `_warn_skipped_large` (semble #252): a silent skip left unexplained
/// gaps in results. `from_env` selects the hint: the env var when the limit
/// was resolved from it, else the `max_file_bytes` option the caller pinned.
pub(crate) fn skipped_large_warning(
    skipped: &[String],
    max_file_bytes: u64,
    from_env: bool,
) -> Option<String> {
    if skipped.is_empty() {
        return None;
    }
    let shown: Vec<String> = skipped.iter().take(5).map(|p| escape_control(p)).collect();
    let knob = if from_env {
        MAX_FILE_BYTES_ENV
    } else {
        "max_file_bytes"
    };
    Some(format!(
        "Skipped {} file(s) exceeding the maximum file size of {} bytes \
         (raise {} to include them): {}{}",
        skipped.len(),
        max_file_bytes,
        knob,
        shown.join(", "),
        if skipped.len() > 5 { " ..." } else { "" },
    ))
}

/// Replace a file's BM25 postings: remove its old slots (if any), then add its
/// new ones.
fn reindex_file(
    bm25_index: &mut Bm25Index,
    indexed_path: &str,
    file_chunks: &[Chunk],
    previous_entry: Option<&FileManifestEntry>,
) -> Result<(), String> {
    if let Some(entry) = previous_entry {
        for slot in 0..entry.count {
            bm25_index.remove_document(&make_chunk_id(indexed_path, slot));
        }
    }
    for (slot, chunk) in file_chunks.iter().enumerate() {
        bm25_index.add_document(
            &make_chunk_id(indexed_path, slot),
            &tokenize(&enrich_for_bm25(chunk)),
        )?;
    }
    Ok(())
}

/// Split a previous index into the parts the rebuild consumes: its BM25 index
/// is mutated in place, and its chunk / vector rows are wrapped in `Option` so
/// unchanged files can move them out without copying.
type PreviousParts = (
    Bm25Index,
    FileManifest,
    Vec<Option<Chunk>>,
    Vec<Option<Vec<f32>>>,
);

fn open_previous(previous: Option<PreviousIndex>) -> PreviousParts {
    match previous {
        Some(prev) => (
            prev.bm25_index,
            prev.files,
            prev.chunks.into_iter().map(Some).collect(),
            prev.vectors.into_iter().map(Some).collect(),
        ),
        None => (
            Bm25Index::new(),
            FileManifest::new(),
            Vec::new(),
            Vec::new(),
        ),
    }
}

/// The path a file is indexed under: relative to `display_root` when set.
fn display_path(file_path: &Path, display_root: Option<&Path>) -> String {
    match display_root {
        Some(root) => file_path
            .strip_prefix(root)
            .unwrap_or(file_path)
            .to_string_lossy()
            .into_owned(),
        None => file_path.to_string_lossy().into_owned(),
    }
}

/// Move an unchanged file's previous chunk + vector rows out, or `None` when
/// the file is new, changed, or its manifest range is out of bounds. Each row
/// is taken at most once because a validated manifest's ranges never overlap.
fn take_previous_rows(
    entry: Option<&FileManifestEntry>,
    hash: &str,
    previous_chunks: &mut [Option<Chunk>],
    previous_vectors: &mut [Option<Vec<f32>>],
) -> Option<(Vec<Chunk>, Vec<Vec<f32>>)> {
    let entry = entry?;
    if entry.hash != hash
        || entry.end() > previous_chunks.len()
        || entry.end() > previous_vectors.len()
    {
        return None;
    }
    let rows: Option<Vec<Chunk>> = previous_chunks[entry.start..entry.end()]
        .iter_mut()
        .map(Option::take)
        .collect();
    let vecs: Option<Vec<Vec<f32>>> = previous_vectors[entry.start..entry.end()]
        .iter_mut()
        .map(Option::take)
        .collect();
    rows.zip(vecs)
}

/// Fill the `None` holes left for freshly chunked files with one batched embed
/// — the tokenizer parallelises per batch, so a call per file would serialise a
/// cold build. Fresh rows are normalised through the backend so they match the
/// reused (already-normalised) rows.
fn embed_fresh_rows(
    model: &Model,
    chunks: &[Chunk],
    fresh_rows: &[usize],
    mut vectors: Vec<Option<Vec<f32>>>,
) -> Result<Vec<Vec<f32>>, String> {
    let fresh_chunks: Vec<&Chunk> = fresh_rows.iter().map(|&i| &chunks[i]).collect();
    let fresh_vectors =
        SelectableBasicBackend::from_vectors(embed_chunk_refs(model, &fresh_chunks))?.vectors;
    if fresh_vectors.len() != fresh_rows.len() {
        return Err("Embedder returned the wrong number of rows".to_string());
    }
    for (&row, vector) in fresh_rows.iter().zip(fresh_vectors) {
        vectors[row] = Some(vector);
    }
    let vectors: Option<Vec<Vec<f32>>> = vectors.into_iter().collect();
    vectors.ok_or_else(|| "Internal error: an embedding row was left unfilled".to_string())
}

/// Create an index from a resolved directory, optionally reusing a previous
/// index's unchanged files. Errors when no chunks are produced.
pub fn create_index_from_path(
    path: &Path,
    options: &CreateIndexOptions,
    previous: Option<PreviousIndex>,
) -> Result<CreateIndexResult, String> {
    let content = options
        .content
        .clone()
        .unwrap_or_else(|| vec![ContentType::Code]);
    let resolved = get_extensions(&content, options.extensions.as_deref());
    let ext_refs: Vec<&str> = resolved.iter().map(String::as_str).collect();
    let max_file_bytes = options.max_file_bytes.unwrap_or_else(get_max_file_bytes);

    let (mut bm25_index, previous_files, mut previous_chunks, mut previous_vectors) =
        open_previous(previous);

    let mut chunks: Vec<Chunk> = Vec::new();
    let mut chunk_ids: Vec<String> = Vec::new();
    // Reused rows land here directly; freshly chunked files leave a `None` hole
    // that the single batched embed pass below fills, so the tokenizer keeps the
    // whole-corpus batching it had before incremental reuse existed.
    let mut vectors: Vec<Option<Vec<f32>>> = Vec::new();
    let mut fresh_rows: Vec<usize> = Vec::new();
    let mut files = FileManifest::new();
    let mut skipped_large: Vec<String> = Vec::new();

    for file_path in walk_files(path, &ext_refs, &[]) {
        let language = detect_language(&file_path.to_string_lossy());
        let size = match std::fs::metadata(&file_path) {
            Ok(meta) => meta.len(),
            Err(_) => continue,
        };
        if size > max_file_bytes {
            skipped_large.push(display_path(&file_path, options.display_root.as_deref()));
            continue;
        }
        let Ok(bytes) = std::fs::read(&file_path) else {
            continue;
        };
        let hash = sha256_hex(&bytes);
        let indexed_path = display_path(&file_path, options.display_root.as_deref());
        // `to_string_lossy` is not injective: on Unix, file names that differ
        // only in invalid UTF-8 bytes collapse to the same display path, and the
        // BM25 chunk ids derived from it would then collide and abort the whole
        // build. Keep the first such file and skip the rest, as a valid UTF-8
        // tree can never hit this.
        if files.contains_key(&indexed_path) {
            eprintln!(
                "csp: skipping {}: its display path collides with an already indexed file \
                 (non-UTF-8 file name)",
                escape_control(&file_path.display().to_string())
            );
            continue;
        }
        let previous_entry = previous_files.get(&indexed_path);

        let reused = take_previous_rows(
            previous_entry,
            &hash,
            &mut previous_chunks,
            &mut previous_vectors,
        );
        let start = chunks.len();
        let file_chunks = match reused {
            Some((file_chunks, file_vectors)) => {
                vectors.extend(file_vectors.into_iter().map(Some));
                file_chunks
            }
            None => {
                // Lossy UTF-8 decode (invalid bytes → U+FFFD): only an IO error
                // skips a file, never an encoding error.
                let source = String::from_utf8_lossy(&bytes).into_owned();
                let file_chunks = chunk_source(&source, &indexed_path, language);
                reindex_file(&mut bm25_index, &indexed_path, &file_chunks, previous_entry)?;
                fresh_rows.extend(start..start + file_chunks.len());
                vectors.extend(std::iter::repeat_n(None, file_chunks.len()));
                file_chunks
            }
        };

        let count = file_chunks.len();
        chunk_ids.extend((0..count).map(|slot| make_chunk_id(&indexed_path, slot)));
        chunks.extend(file_chunks);
        files.insert(indexed_path, FileManifestEntry { hash, start, count });
    }

    // Files that vanished since the previous index: drop their postings.
    for (indexed_path, entry) in &previous_files {
        if !files.contains_key(indexed_path) {
            reindex_file(&mut bm25_index, indexed_path, &[], Some(entry))?;
        }
    }

    // Warn before the empty check so a tree of only oversized files explains
    // itself rather than just reporting "no supported files".
    if let Some(warning) = skipped_large_warning(
        &skipped_large,
        max_file_bytes,
        options.max_file_bytes.is_none(),
    ) {
        eprintln!("csp: {warning}");
    }

    if chunks.is_empty() {
        return Err(format!(
            "No supported files found under {}.",
            path.display()
        ));
    }

    let vectors = embed_fresh_rows(options.model, &chunks, &fresh_rows, vectors)?;

    bm25_index.set_doc_order(chunk_ids);
    let semantic_index = SelectableBasicBackend::from_normalized(vectors)?;

    Ok(CreateIndexResult {
        bm25_index,
        semantic_index,
        chunks,
        files,
    })
}

#[cfg(test)]
mod tests;
