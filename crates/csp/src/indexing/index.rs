//! `CspIndex` — the hybrid (dense + BM25) search orchestrator. Port of
//! `src/indexing/index.ts` (← semble `index/index.py`).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::chunking::source::DESIRED_CHUNK_LENGTH_CHARS;
use crate::indexing::create::{create_index_from_path, CreateIndexOptions};
use crate::indexing::dense::{load_model, make_stub_model, Model, SelectableBasicBackend};
use crate::indexing::sparse::Bm25Index;
use crate::search::{search as run_search, SearchOptions as RunSearchOptions, SearchResult};
use crate::types::{chunk_from_dict, chunk_to_dict, Chunk, ChunkDict, ContentType, IndexStats};

/// On-disk index schema version.
pub const INDEX_SCHEMA_VERSION: u32 = 1;

/// Default content selection (code-only).
pub const DEFAULT_CONTENT: &[ContentType] = &[ContentType::Code];

/// Default result count when `top_k` is omitted.
const DEFAULT_TOP_K: usize = 5;

/// Persisted index manifest tying the on-disk artifacts together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexManifest {
    pub schema_version: u32,
    pub content_hash: String,
    pub source_id: Option<String>,
    pub content: Vec<ContentType>,
    pub model_id: String,
    /// Runtime model implementation used to build vectors (`static` or `stub`).
    /// Absent in legacy manifests, which are conservatively treated as stale.
    pub model_kind: Option<String>,
    /// Target chunk length the index was built with. Changing it alters every
    /// chunk boundary, so a cache built with a different value must be rebuilt
    /// (mirrors semble `_metadata_matches`). `None` = built before this field
    /// existed → treated as a mismatch.
    pub chunk_size: Option<u32>,
}

/// Query options for [`CspIndex::search`] / [`CspIndex::find_related`].
#[derive(Debug, Clone, Default)]
pub struct QueryOptions {
    pub top_k: Option<usize>,
    pub filter_languages: Option<Vec<String>>,
    pub filter_paths: Option<Vec<String>>,
}

/// Build/load options shared by `from_path` / `from_git`.
#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    pub model_path: Option<String>,
    pub content: Option<Vec<ContentType>>,
}

/// Fully built index state.
pub struct CspIndexState {
    pub model: Model,
    pub bm25_index: Bm25Index,
    pub semantic_index: SelectableBasicBackend,
    pub chunks: Vec<Chunk>,
    pub model_path: String,
    pub root: Option<String>,
    pub content: Vec<ContentType>,
}

/// Hybrid (dense + BM25) code search index.
#[derive(Debug)]
pub struct CspIndex {
    pub model: Model,
    pub bm25_index: Bm25Index,
    pub semantic_index: SelectableBasicBackend,
    pub chunks: Vec<Chunk>,
    pub model_path: String,
    pub root: Option<String>,
    pub content: Vec<ContentType>,
    /// Per-file character counts (repo-relative path → UTF-16 length) captured
    /// at build time from the source tree, for token-savings telemetry. Empty
    /// when the source files aren't available (e.g. a git index loaded from
    /// cache). Derived metadata, not part of [`CspIndexState`].
    pub file_sizes: HashMap<String, u64>,
}

pub(crate) fn normalize_content(content: Option<Vec<ContentType>>) -> Vec<ContentType> {
    content.unwrap_or_else(|| DEFAULT_CONTENT.to_vec())
}

impl CspIndex {
    pub fn new(state: CspIndexState) -> Self {
        Self {
            model: state.model,
            bm25_index: state.bm25_index,
            semantic_index: state.semantic_index,
            chunks: state.chunks,
            model_path: state.model_path,
            root: state.root,
            content: state.content,
            file_sizes: HashMap::new(),
        }
    }

    /// Build an index from a local directory.
    pub fn from_path(path: &Path, options: &LoadOptions) -> Result<Self, String> {
        let meta = std::fs::metadata(path)
            .map_err(|_| format!("Path does not exist: {}", path.display()))?;
        if !meta.is_dir() {
            return Err(format!("Path is not a directory: {}", path.display()));
        }

        let (model, model_path) = load_model(options.model_path.as_deref());
        let content = normalize_content(options.content.clone());

        let result = create_index_from_path(
            path,
            &CreateIndexOptions {
                model: &model,
                extensions: None,
                content: Some(content.clone()),
                display_root: Some(path.to_path_buf()),
            },
        )?;

        let mut index = Self::new(CspIndexState {
            model,
            bm25_index: result.bm25_index,
            semantic_index: result.semantic_index,
            chunks: result.chunks,
            model_path,
            // Absolute, like upstream's `path.resolve()`, so an index built from
            // `.` still finds its source tree when loaded from another cwd.
            root: Some(
                std::path::absolute(path)
                    .unwrap_or_else(|_| path.to_path_buf())
                    .to_string_lossy()
                    .into_owned(),
            ),
            content,
        });
        // Capture file sizes now, while the source tree is on disk.
        index.file_sizes = compute_file_sizes(path, &index.chunks);
        Ok(index)
    }

    /// Build an index from a remote git URL (shallow clone into a temp dir).
    pub fn from_git(
        url: &str,
        options: &LoadOptions,
        git_ref: Option<&str>,
    ) -> Result<Self, String> {
        let dir = tempfile::Builder::new()
            .prefix("csp-git-")
            .tempdir()
            .map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700));
        }

        clone_shallow(url, dir.path(), git_ref)?;
        let index = Self::from_path(dir.path(), options)?;
        // `from_path` already captured file sizes from the checkout; carry them
        // over since the temp dir is removed when `dir` drops.
        let file_sizes = index.file_sizes.clone();
        // Re-root at the URL so a persisted manifest records a stable sourceId
        // (the temp checkout is removed when `dir` drops).
        let mut rerooted = Self::new(CspIndexState {
            model: index.model,
            bm25_index: index.bm25_index,
            semantic_index: index.semantic_index,
            chunks: index.chunks,
            model_path: index.model_path,
            root: Some(url.to_string()),
            content: index.content,
        });
        rerooted.file_sizes = file_sizes;
        Ok(rerooted)
    }

    /// Aggregate index statistics.
    pub fn stats(&self) -> IndexStats {
        let mut files: HashSet<&str> = HashSet::new();
        let mut languages: BTreeMap<String, usize> = BTreeMap::new();
        for chunk in &self.chunks {
            files.insert(chunk.file_path.as_str());
            if let Some(lang) = &chunk.language {
                *languages.entry(lang.clone()).or_insert(0) += 1;
            }
        }
        IndexStats {
            indexed_files: files.len(),
            total_chunks: self.chunks.len(),
            languages,
        }
    }

    /// Hybrid search over the indexed chunks. Returns `[]` for blank queries,
    /// non-positive `top_k`, an empty index, or filters that match nothing.
    pub fn search(&self, query: &str, options: &QueryOptions) -> Vec<SearchResult> {
        let top_k = options.top_k.unwrap_or(DEFAULT_TOP_K);
        if query.trim().is_empty() || top_k == 0 || self.chunks.is_empty() {
            return Vec::new();
        }

        let selector = self.build_selector(options);
        if let Some(sel) = &selector {
            if sel.is_empty() {
                return Vec::new();
            }
        }

        run_search(
            query,
            &self.model,
            &self.semantic_index,
            &self.bm25_index,
            &self.chunks,
            top_k,
            &RunSearchOptions {
                alpha: None,
                selector,
                rerank: None,
            },
        )
    }

    /// Find chunks similar to a seed, excluding the seed itself.
    pub fn find_related(&self, seed: &Chunk, options: &QueryOptions) -> Vec<SearchResult> {
        let top_k = options.top_k.unwrap_or(DEFAULT_TOP_K);
        if top_k == 0 || self.chunks.is_empty() {
            return Vec::new();
        }

        let query_embedding = self.model.encode(std::slice::from_ref(&seed.content));
        let batch = self
            .semantic_index
            .query(&query_embedding, top_k + 1, None)
            .unwrap_or_default();
        let Some(first) = batch.into_iter().next() else {
            return Vec::new();
        };

        let mut results = Vec::new();
        for (index, distance) in first {
            let Some(chunk) = self.chunks.get(index) else {
                continue;
            };
            if chunk == seed {
                continue;
            }
            results.push(SearchResult {
                chunk: chunk.clone(),
                score: 1.0 - distance,
            });
            if results.len() >= top_k {
                break;
            }
        }
        results
    }

    /// Build a candidate-index selector from filters, or `None` when none set.
    /// An empty `Vec` (filters matched nothing) is returned as-is.
    fn build_selector(&self, options: &QueryOptions) -> Option<Vec<u32>> {
        let lang_filter = options.filter_languages.as_ref().filter(|l| !l.is_empty());
        let path_filter = options.filter_paths.as_ref().filter(|p| !p.is_empty());
        if lang_filter.is_none() && path_filter.is_none() {
            return None;
        }

        let mut indices = Vec::new();
        for (i, chunk) in self.chunks.iter().enumerate() {
            if let Some(langs) = lang_filter {
                let lang = chunk.language.as_deref().unwrap_or("");
                if !langs.iter().any(|l| l == lang) {
                    continue;
                }
            }
            if let Some(paths) = path_filter {
                if !paths.iter().any(|p| chunk.file_path.contains(p.as_str())) {
                    continue;
                }
            }
            indices.push(i as u32);
        }
        Some(indices)
    }

    /// Persist the index to `dir` (chunks.json / bm25.json / vectors.bin /
    /// args.json / manifest.json). `content_hash` overrides the manifest hash.
    pub fn save(&self, dir: &Path, content_hash: Option<&str>) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;

        let serialized: Vec<ChunkDict> = self.chunks.iter().map(chunk_to_dict).collect();
        let chunks_json = serde_json::to_string(&serialized).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("chunks.json"), &chunks_json).map_err(|e| e.to_string())?;

        self.bm25_index.save(dir).map_err(|e| e.to_string())?;
        self.semantic_index.save(dir).map_err(|e| e.to_string())?;

        let manifest = IndexManifest {
            schema_version: INDEX_SCHEMA_VERSION,
            content_hash: content_hash
                .map(str::to_string)
                .unwrap_or_else(|| hash_chunks(&chunks_json)),
            source_id: self.root.clone(),
            content: self.content.clone(),
            model_id: self.model_path.clone(),
            model_kind: Some(self.model.kind().to_string()),
            chunk_size: Some(DESIRED_CHUNK_LENGTH_CHARS as u32),
        };
        let manifest_json = serde_json::to_string(&manifest).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("manifest.json"), manifest_json).map_err(|e| e.to_string())
    }

    /// Load an index previously persisted with [`save`](Self::save).
    pub fn load_from_disk(dir: &Path) -> Result<Self, String> {
        if !dir.exists() {
            return Err(format!("Index not found: {}", dir.display()));
        }
        for name in [
            "manifest.json",
            "chunks.json",
            "bm25.json",
            "vectors.bin",
            "args.json",
        ] {
            if !dir.join(name).exists() {
                return Err(format!("Missing: {}", dir.join(name).display()));
            }
        }

        let raw = std::fs::read_to_string(dir.join("manifest.json")).map_err(|e| e.to_string())?;
        let value: serde_json::Value = serde_json::from_str(&raw).map_err(|e| e.to_string())?;
        let version = value
            .get("schemaVersion")
            .and_then(serde_json::Value::as_u64);
        if version != Some(u64::from(INDEX_SCHEMA_VERSION)) {
            return Err(format!(
                "Index schema version mismatch: expected {INDEX_SCHEMA_VERSION}, got {}",
                version.map_or_else(|| "undefined".to_string(), |v| v.to_string())
            ));
        }
        let manifest = parse_manifest(&value)?;

        let chunks_raw =
            std::fs::read_to_string(dir.join("chunks.json")).map_err(|e| e.to_string())?;
        let chunk_values: Vec<serde_json::Value> =
            serde_json::from_str(&chunks_raw).map_err(|e| e.to_string())?;
        let mut chunks = Vec::with_capacity(chunk_values.len());
        for v in &chunk_values {
            chunks.push(chunk_from_dict(v).map_err(|e| e.to_string())?);
        }

        let bm25_index = Bm25Index::load(dir).map_err(|e| e.to_string())?;
        let semantic_index = SelectableBasicBackend::load(dir)?;

        let (model, model_path) = load_model(Some(&manifest.model_id));
        // Align the query model's dim with the persisted vectors.
        let model = if model.dim() == semantic_index.dim {
            model
        } else {
            make_stub_model(semantic_index.dim)
        };

        let mut index = Self::new(CspIndexState {
            model,
            bm25_index,
            semantic_index,
            chunks,
            model_path,
            root: manifest.source_id,
            content: manifest.content,
        });
        // Recompute file sizes from the source when it's a still-present local
        // directory (mirrors semble reading sizes off `root` on load). A git URL
        // or a moved source leaves this empty → `file_chars` is simply 0.
        if let Some(root) = index.root.as_deref() {
            let root_path = Path::new(root);
            if root_path.is_dir() {
                index.file_sizes = compute_file_sizes(root_path, &index.chunks);
            }
        }
        Ok(index)
    }
}

/// Per-file UTF-16 character counts for the unique files referenced by `chunks`,
/// read from `root`. Mirrors semble `_compute_file_sizes` (unreadable files are
/// skipped). Feeds the `file_chars` side of token-savings telemetry; UTF-16 keeps
/// it consistent with `stats::save_search_stats`'s snippet accounting.
///
/// Chunk paths are repo-relative by construction; a path that is absolute or
/// escapes `root` via `..` can only come from a tampered on-disk index, so it is
/// skipped rather than resolved (path traversal guard — a deliberate addition
/// over upstream, which joins the path unchecked). Only regular files are read:
/// the file walker never follows symlinks, and a path that has since become a
/// symlink, FIFO, or device must not be able to redirect or stall the read.
fn compute_file_sizes(root: &Path, chunks: &[Chunk]) -> HashMap<String, u64> {
    let mut sizes: HashMap<String, u64> = HashMap::new();
    for chunk in chunks {
        if sizes.contains_key(&chunk.file_path) {
            continue;
        }
        let rel = Path::new(&chunk.file_path);
        if !is_safe_relative_path(rel) {
            continue;
        }
        let full = root.join(rel);
        let is_regular_file = std::fs::symlink_metadata(&full)
            .map(|m| m.is_file())
            .unwrap_or(false);
        if !is_regular_file {
            continue;
        }
        if let Ok(text) = std::fs::read_to_string(&full) {
            sizes.insert(chunk.file_path.clone(), text.encode_utf16().count() as u64);
        }
    }
    sizes
}

/// `true` when `path` is relative and contains no `..` or root component, so
/// joining it onto an index root cannot resolve outside that root.
fn is_safe_relative_path(path: &Path) -> bool {
    use std::path::Component;
    !path.is_absolute()
        && !path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

/// Shallow-clone `url` into `dir`, non-interactively. Rejects a ref starting
/// with `-` (git-flag injection, CWE-88).
fn clone_shallow(url: &str, dir: &Path, git_ref: Option<&str>) -> Result<(), String> {
    if let Some(r) = git_ref {
        if r.starts_with('-') {
            return Err(format!("Invalid git ref (must not start with '-'): {r}"));
        }
    }

    let mut cmd = Command::new("git");
    cmd.args(["clone", "--depth", "1"]);
    if let Some(r) = git_ref {
        cmd.args(["--branch", r]);
    }
    cmd.arg("--").arg(url).arg(dir);
    cmd.env("GIT_TERMINAL_PROMPT", "0");

    let output = cmd
        .output()
        .map_err(|e| format!("git clone failed for {url}: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        let detail = if detail.is_empty() {
            "unknown error"
        } else {
            detail
        };
        return Err(format!("git clone failed for {url}: {detail}"));
    }
    Ok(())
}

/// Deterministic sha256 (hex) of the serialized chunks JSON.
fn hash_chunks(chunks_json: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(chunks_json.as_bytes());
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Parse and validate a persisted manifest (an on-disk trust boundary).
pub fn parse_manifest(raw: &serde_json::Value) -> Result<IndexManifest, String> {
    let obj = raw.as_object().ok_or("Invalid manifest: not an object")?;

    let schema_version = obj
        .get("schemaVersion")
        .and_then(serde_json::Value::as_u64)
        .ok_or("Invalid manifest: schemaVersion must be a number")?;
    let content_hash = obj
        .get("contentHash")
        .and_then(serde_json::Value::as_str)
        .ok_or("Invalid manifest: contentHash must be a string")?
        .to_string();
    let source_id = match obj.get("sourceId") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(_) => return Err("Invalid manifest: sourceId must be a string or null".to_string()),
    };
    let model_id = obj
        .get("modelId")
        .and_then(serde_json::Value::as_str)
        .ok_or("Invalid manifest: modelId must be a string")?
        .to_string();
    let model_kind = match obj.get("modelKind") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(kind)) if matches!(kind.as_str(), "static" | "stub") => {
            Some(kind.clone())
        }
        Some(_) => {
            return Err("Invalid manifest: modelKind must be 'static', 'stub', or null".to_string())
        }
    };
    // Absent/null = built before the field existed → None (treated as a cache
    // mismatch by `try_reuse`). A present-but-non-numeric value is malformed.
    let chunk_size = obj
        .get("chunkSize")
        .filter(|v| !v.is_null())
        .map(|v| {
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or("Invalid manifest: chunkSize must be a u32")
        })
        .transpose()?;
    let content_arr = obj
        .get("content")
        .and_then(serde_json::Value::as_array)
        .ok_or("Invalid manifest: content must be an array of ContentType")?;
    let mut content = Vec::with_capacity(content_arr.len());
    for item in content_arr {
        let parsed: ContentType = serde_json::from_value(item.clone())
            .map_err(|_| "Invalid manifest: content must be an array of ContentType".to_string())?;
        content.push(parsed);
    }

    Ok(IndexManifest {
        schema_version: u32::try_from(schema_version)
            .map_err(|_| "Invalid manifest: schemaVersion out of range")?,
        content_hash,
        source_id,
        content,
        model_id,
        model_kind,
        chunk_size,
    })
}

pub use crate::indexing::cache_orchestrator::{
    load_or_build_index, source_fingerprint, LoadOrBuildOptions,
};

#[cfg(test)]
mod tests;
