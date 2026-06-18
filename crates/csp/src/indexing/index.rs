//! `CspIndex` — the hybrid (dense + BM25) search orchestrator. Port of
//! `src/indexing/index.ts` (← semble `index/index.py`), plus the
//! `load_or_build_index` cache orchestration from `src/indexing/cache.ts`.

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::indexing::cache::{
    compute_content_hash, ensure_cache_dir, resolve_cache_dir, CacheFile, CacheLocation,
};
use crate::indexing::create::{create_index_from_path, CreateIndexOptions, MAX_FILE_BYTES};
use crate::indexing::dense::{load_model, make_stub_model, Model, SelectableBasicBackend};
use crate::indexing::file_walker::walk_files;
use crate::indexing::files::get_extensions;
use crate::indexing::sparse::Bm25Index;
use crate::search::{search as run_search, SearchOptions as RunSearchOptions, SearchResult};
use crate::types::{chunk_from_dict, chunk_to_dict, Chunk, ChunkDict, ContentType, IndexStats};
use crate::utils::is_git_url;

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
}

fn normalize_content(content: Option<Vec<ContentType>>) -> Vec<ContentType> {
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

        Ok(Self::new(CspIndexState {
            model,
            bm25_index: result.bm25_index,
            semantic_index: result.semantic_index,
            chunks: result.chunks,
            model_path,
            root: Some(path.to_string_lossy().into_owned()),
            content,
        }))
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
        // Re-root at the URL so a persisted manifest records a stable sourceId
        // (the temp checkout is removed when `dir` drops).
        Ok(Self::new(CspIndexState {
            model: index.model,
            bm25_index: index.bm25_index,
            semantic_index: index.semantic_index,
            chunks: index.chunks,
            model_path: index.model_path,
            root: Some(url.to_string()),
            content: index.content,
        }))
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
        let model = if model.dim == semantic_index.dim {
            model
        } else {
            make_stub_model(semantic_index.dim)
        };

        Ok(Self::new(CspIndexState {
            model,
            bm25_index,
            semantic_index,
            chunks,
            model_path,
            root: manifest.source_id,
            content: manifest.content,
        }))
    }
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
    })
}

// --- load_or_build_index (cache.ts orchestration) ---------------------------

/// Options for [`load_or_build_index`].
#[derive(Debug, Clone, Default)]
pub struct LoadOrBuildOptions {
    pub base_dir: Option<std::path::PathBuf>,
    pub git_ref: Option<String>,
    pub content: Option<Vec<ContentType>>,
    pub model_path: Option<String>,
}

/// Collect the source files `from_path` would index, as [`CacheFile`] entries.
fn collect_source_files(root: &Path, content: &[ContentType]) -> Vec<CacheFile> {
    let resolved = get_extensions(content, None);
    let ext_refs: Vec<&str> = resolved.iter().map(String::as_str).collect();
    let mut files = Vec::new();
    for file_path in walk_files(root, &ext_refs, &[]) {
        let Ok(meta) = std::fs::metadata(&file_path) else {
            continue;
        };
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let Ok(raw) = std::fs::read(&file_path) else {
            continue;
        };
        let rel = file_path.strip_prefix(root).unwrap_or(&file_path);
        files.push(CacheFile {
            path: rel.to_string_lossy().into_owned(),
            content: raw,
        });
    }
    files
}

/// Load a cached index for `source` if fresh, else build, persist, and return.
pub fn load_or_build_index(source: &str, options: &LoadOrBuildOptions) -> Result<CspIndex, String> {
    let content = normalize_content(options.content.clone());
    let is_git = is_git_url(source);

    let location = CacheLocation {
        base_dir: options.base_dir.clone(),
        git_ref: options.git_ref.clone(),
    };
    let cache_dir = resolve_cache_dir(source, &content, &location);
    let base_only = CacheLocation {
        base_dir: options.base_dir.clone(),
        git_ref: None,
    };
    ensure_cache_dir(&cache_dir, &base_only);

    // Local sources: the source-file hash is the cache-validity oracle. Git
    // sources are URL+ref keyed (no cheap live hash).
    let source_hash = if is_git {
        None
    } else {
        Some(compute_content_hash(&collect_source_files(
            Path::new(source),
            &content,
        )))
    };

    if let Some(cached) = try_reuse(&cache_dir, is_git, source_hash.as_deref()) {
        return Ok(cached);
    }

    let load_options = LoadOptions {
        model_path: options.model_path.clone(),
        content: Some(content),
    };
    let index = if is_git {
        CspIndex::from_git(source, &load_options, options.git_ref.as_deref())?
    } else {
        CspIndex::from_path(Path::new(source), &load_options)?
    };
    index.save(&cache_dir, source_hash.as_deref())?;
    Ok(index)
}

/// Reuse a cached index when present and valid, else `None`.
fn try_reuse(cache_dir: &Path, is_git: bool, source_hash: Option<&str>) -> Option<CspIndex> {
    let manifest_path = cache_dir.join("manifest.json");
    if !manifest_path.exists() {
        return None;
    }
    if !is_git {
        let raw = std::fs::read_to_string(&manifest_path).ok()?;
        let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
        let manifest = parse_manifest(&value).ok()?;
        if Some(manifest.content_hash.as_str()) != source_hash {
            return None;
        }
    }
    CspIndex::load_from_disk(cache_dir).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexing::dense::make_stub_model;
    use tempfile::tempdir;

    fn make_chunk(
        file_path: &str,
        start: u32,
        end: u32,
        language: Option<&str>,
        content: &str,
    ) -> Chunk {
        Chunk {
            content: content.to_string(),
            file_path: file_path.to_string(),
            start_line: start,
            end_line: end,
            language: language.map(str::to_string),
        }
    }

    fn build_index(chunks: Vec<Chunk>) -> CspIndex {
        let model = make_stub_model(4);
        let vectors: Vec<Vec<f32>> = (0..chunks.len())
            .map(|i| {
                let mut v = vec![0f32; 4];
                v[0] = (i + 1) as f32;
                v
            })
            .collect();
        CspIndex::new(CspIndexState {
            model,
            bm25_index: Bm25Index::build(&vec![vec!["x".to_string()]; chunks.len()]),
            semantic_index: SelectableBasicBackend::from_vectors(vectors).unwrap(),
            chunks,
            model_path: "test-model".to_string(),
            root: None,
            content: DEFAULT_CONTENT.to_vec(),
        })
    }

    #[test]
    fn stats_zero_for_empty() {
        let idx = build_index(vec![]);
        let stats = idx.stats();
        assert_eq!(stats.indexed_files, 0);
        assert_eq!(stats.total_chunks, 0);
        assert!(stats.languages.is_empty());
    }

    #[test]
    fn stats_reflect_distribution() {
        let chunks = vec![
            make_chunk("a.ts", 1, 10, Some("typescript"), "x"),
            make_chunk("a.ts", 11, 20, Some("typescript"), "y"),
            make_chunk("b.py", 1, 5, Some("python"), "z"),
            make_chunk("c.bin", 1, 1, None, "w"),
        ];
        let stats = build_index(chunks).stats();
        assert_eq!(stats.indexed_files, 3);
        assert_eq!(stats.total_chunks, 4);
        assert_eq!(stats.languages.get("typescript"), Some(&2));
        assert_eq!(stats.languages.get("python"), Some(&1));
        assert_eq!(stats.languages.len(), 2);
    }

    #[test]
    fn search_empty_query_and_index() {
        let idx = build_index(vec![make_chunk("a.ts", 1, 1, Some("typescript"), "x")]);
        assert!(idx.search("", &QueryOptions::default()).is_empty());
        assert!(idx.search("   ", &QueryOptions::default()).is_empty());
        let empty = build_index(vec![]);
        assert!(empty
            .search("anything", &QueryOptions::default())
            .is_empty());
    }

    #[test]
    fn search_top_k_zero() {
        let idx = build_index(vec![make_chunk("a.ts", 1, 1, Some("typescript"), "x")]);
        let opts = QueryOptions {
            top_k: Some(0),
            ..Default::default()
        };
        assert!(idx.search("anything", &opts).is_empty());
    }

    #[test]
    fn search_filters_matching_nothing() {
        let chunks = vec![
            make_chunk("a.ts", 1, 10, Some("typescript"), "alpha"),
            make_chunk("b.py", 1, 10, Some("python"), "beta"),
        ];
        let idx = build_index(chunks);
        let lang_opts = QueryOptions {
            filter_languages: Some(vec!["nonexistent".to_string()]),
            ..Default::default()
        };
        assert!(idx.search("anything", &lang_opts).is_empty());
        let path_opts = QueryOptions {
            filter_paths: Some(vec!["nope.ts".to_string()]),
            ..Default::default()
        };
        assert!(idx.search("anything", &path_opts).is_empty());
    }

    #[test]
    fn find_related_excludes_seed() {
        let chunks = vec![
            make_chunk("a.ts", 1, 10, Some("typescript"), "seed chunk"),
            make_chunk("a.ts", 11, 20, Some("typescript"), "companion 1"),
            make_chunk("b.ts", 1, 5, Some("typescript"), "companion 2"),
        ];
        let idx = build_index(chunks.clone());
        let opts = QueryOptions {
            top_k: Some(5),
            ..Default::default()
        };
        let results = idx.find_related(&chunks[0], &opts);
        assert!(!results.iter().any(|r| r.chunk == chunks[0]));
        assert!(results.len() <= 5);
    }

    #[test]
    fn save_load_roundtrip() {
        let chunks = vec![
            make_chunk("a.ts", 1, 10, Some("typescript"), "A"),
            make_chunk("b.ts", 1, 5, Some("python"), "B"),
        ];
        let idx = build_index(chunks);
        let dir = tempdir().unwrap();
        idx.save(dir.path(), None).unwrap();
        let loaded = CspIndex::load_from_disk(dir.path()).unwrap();
        assert_eq!(loaded.chunks.len(), 2);
        let paths: Vec<&str> = loaded.chunks.iter().map(|c| c.file_path.as_str()).collect();
        assert_eq!(paths, ["a.ts", "b.ts"]);
        let stats = loaded.stats();
        assert_eq!(stats.total_chunks, 2);
        assert_eq!(stats.languages.get("typescript"), Some(&1));
        assert_eq!(stats.languages.get("python"), Some(&1));
    }

    #[test]
    fn load_missing_directory() {
        let dir = tempdir().unwrap();
        let err = CspIndex::load_from_disk(&dir.path().join("nope")).unwrap_err();
        assert!(err.contains("Index not found"));
    }

    #[test]
    fn load_missing_artifact() {
        let dir = tempdir().unwrap();
        let err = CspIndex::load_from_disk(dir.path()).unwrap_err();
        assert!(err.contains("Missing:"));
    }

    #[test]
    fn load_schema_version_mismatch() {
        let idx = build_index(vec![make_chunk("a.ts", 1, 10, Some("typescript"), "A")]);
        let dir = tempdir().unwrap();
        idx.save(dir.path(), None).unwrap();
        let manifest_path = dir.path().join("manifest.json");
        let raw = std::fs::read_to_string(&manifest_path).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        value["schemaVersion"] = serde_json::json!(999);
        std::fs::write(&manifest_path, value.to_string()).unwrap();
        let err = CspIndex::load_from_disk(dir.path()).unwrap_err();
        assert!(err.to_lowercase().contains("schema version"));
    }

    #[test]
    fn load_rejects_invalid_content() {
        let idx = build_index(vec![make_chunk("a.ts", 1, 10, Some("typescript"), "A")]);
        let dir = tempdir().unwrap();
        idx.save(dir.path(), None).unwrap();
        let manifest_path = dir.path().join("manifest.json");
        let raw = std::fs::read_to_string(&manifest_path).unwrap();
        let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        value["content"] = serde_json::json!(["bogus"]);
        std::fs::write(&manifest_path, value.to_string()).unwrap();
        assert!(CspIndex::load_from_disk(dir.path()).is_err());
    }

    #[test]
    fn save_writes_manifest_fields() {
        let chunks = vec![make_chunk("a.ts", 1, 10, Some("typescript"), "A")];
        let idx = build_index(chunks);
        let dir = tempdir().unwrap();
        idx.save(dir.path(), None).unwrap();
        let raw = std::fs::read_to_string(dir.path().join("manifest.json")).unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(value["schemaVersion"], 1);
        assert_eq!(value["modelId"], "test-model");
        assert_eq!(value["content"], serde_json::json!(["code"]));
        assert!(value["contentHash"].as_str().unwrap().len() == 64);
    }

    #[test]
    fn save_deterministic_content_hash() {
        let chunks = vec![make_chunk("a.ts", 1, 10, Some("typescript"), "A")];
        let dir_a = tempdir().unwrap();
        let dir_b = tempdir().unwrap();
        build_index(chunks.clone())
            .save(dir_a.path(), None)
            .unwrap();
        build_index(chunks).save(dir_b.path(), None).unwrap();
        let ha: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir_a.path().join("manifest.json")).unwrap(),
        )
        .unwrap();
        let hb: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(dir_b.path().join("manifest.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(ha["contentHash"], hb["contentHash"]);
    }

    #[test]
    fn from_path_errors_on_missing() {
        let dir = tempdir().unwrap();
        let err =
            CspIndex::from_path(&dir.path().join("nope"), &LoadOptions::default()).unwrap_err();
        assert!(err.contains("Path does not exist"));
    }

    #[test]
    fn from_path_errors_on_file() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("f.ts");
        std::fs::write(&file, "x").unwrap();
        let err = CspIndex::from_path(&file, &LoadOptions::default()).unwrap_err();
        assert!(err.contains("Path is not a directory"));
    }

    #[test]
    fn from_path_builds_index() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("sample.ts"), "export const x = 1\n").unwrap();
        let idx = CspIndex::from_path(dir.path(), &LoadOptions::default()).unwrap();
        assert!(!idx.chunks.is_empty());
        assert_eq!(idx.content, DEFAULT_CONTENT.to_vec());
    }

    // --- from_git ---

    #[test]
    fn from_git_rejects_dash_ref() {
        // No clone runs — the ref guard rejects a flag-injection ref first.
        let err = CspIndex::from_git(
            "file:///nonexistent",
            &LoadOptions::default(),
            Some("--upload-pack=evil"),
        )
        .unwrap_err();
        assert!(err.contains("Invalid git ref"));
    }

    #[test]
    fn from_git_errors_on_bad_url() {
        let dir = tempdir().unwrap();
        let bogus = dir.path().join("not-a-repo");
        let err = CspIndex::from_git(
            &format!("file://{}", bogus.display()),
            &LoadOptions::default(),
            None,
        )
        .unwrap_err();
        assert!(err.contains("git clone failed"));
    }

    #[test]
    fn from_git_clones_and_builds() {
        let repo = tempdir().unwrap();
        let run = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(repo.path())
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()
                .expect("git available")
        };
        if !run(&["init", "-q"]).status.success() {
            return; // git unavailable — skip rather than fail.
        }
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "Test"]);
        run(&["config", "commit.gpgsign", "false"]);
        std::fs::write(repo.path().join("a.ts"), "export const x = 1\n").unwrap();
        run(&["add", "."]);
        run(&["commit", "-q", "-m", "initial"]);

        let url = format!("file://{}", repo.path().display());
        let idx = CspIndex::from_git(&url, &LoadOptions::default(), None).unwrap();
        assert!(!idx.chunks.is_empty());
        assert_eq!(idx.root.as_deref(), Some(url.as_str()));
    }

    // --- load_or_build_index (cache.ts loadOrBuildIndex parity) ---

    #[test]
    fn load_or_build_miss_then_hit_then_invalidate() {
        let home = tempdir().unwrap();
        let src = tempdir().unwrap();
        let base = home.path().join(".csp");
        std::fs::write(
            src.path().join("a.ts"),
            "export function alpha() { return 1 }\n",
        )
        .unwrap();
        let src_str = src.path().to_string_lossy().into_owned();
        let opts = LoadOrBuildOptions {
            base_dir: Some(base.clone()),
            ..Default::default()
        };

        // Miss: builds and writes a manifest.
        let first = load_or_build_index(&src_str, &opts).unwrap();
        assert!(!first.chunks.is_empty());
        let cache_dir = resolve_cache_dir(
            &src_str,
            DEFAULT_CONTENT,
            &CacheLocation {
                base_dir: Some(base.clone()),
                git_ref: None,
            },
        );
        assert!(cache_dir.join("manifest.json").exists());

        // Hit: a second call reuses the cache (same chunk count).
        let second = load_or_build_index(&src_str, &opts).unwrap();
        assert_eq!(second.chunks.len(), first.chunks.len());

        // Invalidation: add a file → content hash changes → rebuild reflects it.
        std::fs::write(
            src.path().join("b.ts"),
            "export function beta() { return 2 }\n",
        )
        .unwrap();
        let third = load_or_build_index(&src_str, &opts).unwrap();
        assert!(third.chunks.iter().any(|c| c.file_path == "b.ts"));
        assert!(third.chunks.len() >= first.chunks.len());
    }
}
