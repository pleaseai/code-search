//! MCP server core — the session index cache, the source-safety layer, and the
//! `search` / `find_related` tool handlers. Port of the verifiable core of
//! `src/mcp/server.ts` (← semble `mcp.py`).
//!
//! The handlers and [`IndexCache`] are transport-agnostic and fully tested here.
//! The rmcp stdio server in `csp-cli` (`mcp_server.rs`) wires these handlers onto
//! the live MCP protocol; this core is kept transport-free so it stays unit-
//! testable. [`IndexCache`] holds `Arc<CspIndex>` so it can be shared across the
//! async server's tokio tasks.

use std::sync::Arc;

use indexmap::IndexMap;
use serde_json::json;

use crate::indexing::index::{load_or_build_index, CspIndex, LoadOrBuildOptions, QueryOptions};
use crate::types::ContentType;
use crate::utils::{format_results, is_git_url, resolve_chunk};

/// Server instructions advertised to MCP clients (preserved for the transport).
pub const SERVER_INSTRUCTIONS: &str = concat!(
    "Instant code search for any local or remote git repository. ",
    "Call `search` to find relevant code; call `find_related` on a result to discover similar code elsewhere. ",
    "Prefer these tools over Grep, Glob, or Read for any question about how code works."
);

/// Maximum number of distinct sources held in the session cache (LRU).
const CACHE_MAX_SIZE: usize = 10;

/// Build-or-reuse seam — defaults to [`load_or_build_index`]; tests inject a stub
/// to count calls and assert git-vs-path routing.
pub trait LoadOrBuild {
    fn load_or_build(
        &self,
        source: &str,
        content: &[ContentType],
        git_ref: Option<&str>,
    ) -> Result<CspIndex, String>;
}

/// Default seam: route through the shared on-disk cache.
pub struct DiskLoadOrBuild;

impl LoadOrBuild for DiskLoadOrBuild {
    fn load_or_build(
        &self,
        source: &str,
        content: &[ContentType],
        git_ref: Option<&str>,
    ) -> Result<CspIndex, String> {
        load_or_build_index(
            source,
            &LoadOrBuildOptions {
                content: Some(content.to_vec()),
                git_ref: git_ref.map(str::to_string),
                ..Default::default()
            },
        )
    }
}

/// Session cache of indexed repos/paths, keyed by source (git URL `@ref`, or the
/// absolutized local path). LRU-bounded to [`CACHE_MAX_SIZE`].
pub struct IndexCache<S: LoadOrBuild = DiskLoadOrBuild> {
    tasks: IndexMap<String, Arc<CspIndex>>,
    content: Vec<ContentType>,
    seam: S,
}

impl IndexCache<DiskLoadOrBuild> {
    /// A cache backed by the real on-disk `load_or_build_index`.
    pub fn new(content: Vec<ContentType>) -> Self {
        Self::with_seam(content, DiskLoadOrBuild)
    }
}

impl<S: LoadOrBuild> IndexCache<S> {
    pub fn with_seam(content: Vec<ContentType>, seam: S) -> Self {
        Self {
            tasks: IndexMap::new(),
            content,
            seam,
        }
    }

    fn compute_key(&self, source: &str, git_ref: Option<&str>) -> String {
        if is_git_url(source) {
            match git_ref {
                Some(r) if !r.is_empty() => format!("{source}@{r}"),
                _ => source.to_string(),
            }
        } else {
            // Absolutize without requiring existence (matches `path.resolve`).
            std::path::absolute(source)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| source.to_string())
        }
    }

    /// Return an index for `source`, building and caching it on first access.
    /// A build failure is not cached (the next call retries).
    pub fn get(&mut self, source: &str, git_ref: Option<&str>) -> Result<Arc<CspIndex>, String> {
        let key = self.compute_key(source, git_ref);

        if let Some(existing) = self.tasks.shift_remove(&key) {
            // Touch for LRU (re-insert at the most-recent end).
            self.tasks.insert(key, existing.clone());
            return Ok(existing);
        }

        // LRU eviction: drop the oldest entry when full.
        if self.tasks.len() >= CACHE_MAX_SIZE {
            self.tasks.shift_remove_index(0);
        }

        let index = Arc::new(self.seam.load_or_build(source, &self.content, git_ref)?);
        self.tasks.insert(key, index.clone());
        Ok(index)
    }

    /// Remove the cached entry for `source`.
    pub fn evict(&mut self, source: &str, git_ref: Option<&str>) {
        let key = self.compute_key(source, git_ref);
        self.tasks.shift_remove(&key);
    }

    /// Number of cached entries.
    pub fn size(&self) -> usize {
        self.tasks.len()
    }
}

/// Resolve a cached index for a repo, rejecting unsafe git transport schemes and
/// missing-source cases with descriptive errors.
pub fn get_index<S: LoadOrBuild>(
    repo: Option<&str>,
    default_source: Option<&str>,
    default_ref: Option<&str>,
    cache: &mut IndexCache<S>,
) -> Result<Arc<CspIndex>, String> {
    if let Some(r) = repo {
        if is_git_url(r) && !r.starts_with("https://") && !r.starts_with("http://") {
            return Err(format!(
                "Only https://, http://, or local directory paths are accepted as `repo`. Got: {}",
                json!(r)
            ));
        }
    }
    // An explicit per-call `repo` carries no ref; `default_ref` applies only when
    // falling back to the server's default source (so `csp mcp <url> --ref X`
    // actually pins the indexed revision instead of being silently ignored).
    let use_default = repo.filter(|s| !s.is_empty()).is_none();
    let source = repo.or(default_source).filter(|s| !s.is_empty());
    let Some(source) = source else {
        return Err("No repo specified and no default index. \
             Pass an https:// or http:// git URL or local directory path as `repo`."
            .to_string());
    };
    let git_ref = if use_default { default_ref } else { None };
    cache
        .get(source, git_ref)
        .map_err(|e| format!("Failed to index {}: {e}", json!(source)))
}

/// `search` tool handler. Returns a JSON string (results or `{error}`), or an
/// error message string on failure (mirroring the TS handler's catch).
pub fn search_tool<S: LoadOrBuild>(
    cache: &mut IndexCache<S>,
    default_source: Option<&str>,
    default_ref: Option<&str>,
    query: &str,
    repo: Option<&str>,
    top_k: usize,
) -> String {
    let index = match get_index(repo, default_source, default_ref, cache) {
        Ok(idx) => idx,
        Err(e) => return e,
    };
    let results = index.search(
        query,
        &QueryOptions {
            top_k: Some(top_k),
            ..Default::default()
        },
    );
    if results.is_empty() {
        json!({ "error": "No results found." }).to_string()
    } else {
        format_results(query, &results).to_string()
    }
}

/// `find_related` tool handler.
pub fn find_related_tool<S: LoadOrBuild>(
    cache: &mut IndexCache<S>,
    default_source: Option<&str>,
    default_ref: Option<&str>,
    file_path: &str,
    line: i64,
    repo: Option<&str>,
    top_k: usize,
) -> String {
    let index = match get_index(repo, default_source, default_ref, cache) {
        Ok(idx) => idx,
        Err(e) => return e,
    };
    // Guard the full u32 range, not just the lower bound — a line number above
    // u32::MAX would otherwise wrap on `as u32` and resolve the wrong chunk.
    let chunk = if (0..=i64::from(u32::MAX)).contains(&line) {
        resolve_chunk(&index.chunks, file_path, line as u32)
    } else {
        None
    };
    let Some(chunk) = chunk else {
        return format!(
            "No chunk found at {file_path}:{line}. \
             Make sure the file is indexed and the line number is within a known chunk."
        );
    };
    let results = index.find_related(
        &chunk.clone(),
        &QueryOptions {
            top_k: Some(top_k),
            ..Default::default()
        },
    );
    if results.is_empty() {
        json!({ "error": format!("No related chunks found for {file_path}:{line}.") }).to_string()
    } else {
        format_results(&format!("Chunks related to {file_path}:{line}"), &results).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexing::dense::make_stub_model;
    use crate::indexing::dense::SelectableBasicBackend;
    use crate::indexing::index::CspIndexState;
    use crate::indexing::sparse::Bm25Index;
    use crate::types::Chunk;
    use std::cell::RefCell;

    fn empty_index() -> CspIndex {
        CspIndex::new(CspIndexState {
            model: make_stub_model(4),
            bm25_index: Bm25Index::build(&[]),
            semantic_index: SelectableBasicBackend::from_vectors(vec![]).unwrap(),
            chunks: vec![],
            model_path: "test".to_string(),
            root: None,
            content: vec![ContentType::Code],
        })
    }

    fn index_with_chunk() -> CspIndex {
        let chunk = Chunk {
            content: "fn main() {}".to_string(),
            file_path: "a.ts".to_string(),
            start_line: 1,
            end_line: 10,
            language: Some("typescript".to_string()),
        };
        CspIndex::new(CspIndexState {
            model: make_stub_model(4),
            bm25_index: Bm25Index::build(&[vec!["main".to_string()]]),
            semantic_index: SelectableBasicBackend::from_vectors(vec![vec![1.0, 0.0, 0.0, 0.0]])
                .unwrap(),
            chunks: vec![chunk],
            model_path: "test".to_string(),
            root: None,
            content: vec![ContentType::Code],
        })
    }

    /// Stub seam: counts git vs path builds, never touches disk.
    struct Stub {
        git_calls: RefCell<usize>,
        path_calls: RefCell<usize>,
        fail: bool,
    }
    impl Stub {
        fn new() -> Self {
            Self {
                git_calls: RefCell::new(0),
                path_calls: RefCell::new(0),
                fail: false,
            }
        }
    }
    impl LoadOrBuild for Stub {
        fn load_or_build(
            &self,
            source: &str,
            _c: &[ContentType],
            _r: Option<&str>,
        ) -> Result<CspIndex, String> {
            if self.fail {
                return Err("boom".to_string());
            }
            if is_git_url(source) {
                *self.git_calls.borrow_mut() += 1;
            } else {
                *self.path_calls.borrow_mut() += 1;
            }
            Ok(empty_index())
        }
    }

    #[test]
    fn cache_reuses_second_call() {
        let mut cache = IndexCache::with_seam(vec![ContentType::Code], Stub::new());
        let first = cache.get("/tmp/repo", None).unwrap();
        let second = cache.get("/tmp/repo", None).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(*cache.seam.path_calls.borrow(), 1);
    }

    #[test]
    fn cache_evict_forces_rebuild() {
        let mut cache = IndexCache::with_seam(vec![ContentType::Code], Stub::new());
        cache.get("/tmp/repo", None).unwrap();
        assert_eq!(*cache.seam.path_calls.borrow(), 1);
        cache.evict("/tmp/repo", None);
        assert_eq!(cache.size(), 0);
        cache.get("/tmp/repo", None).unwrap();
        assert_eq!(*cache.seam.path_calls.borrow(), 2);
    }

    #[test]
    fn cache_lru_evicts_oldest() {
        let mut cache = IndexCache::with_seam(vec![ContentType::Code], Stub::new());
        for i in 0..10 {
            cache.get(&format!("/tmp/repo-{i}"), None).unwrap();
        }
        assert_eq!(cache.size(), 10);
        cache.get("/tmp/repo-10", None).unwrap();
        assert_eq!(cache.size(), 10);
        // repo-0 (oldest) was evicted → re-getting it rebuilds.
        let before = *cache.seam.path_calls.borrow();
        cache.get("/tmp/repo-0", None).unwrap();
        assert_eq!(*cache.seam.path_calls.borrow(), before + 1);
    }

    #[test]
    fn cache_git_vs_path_routing() {
        let mut cache = IndexCache::with_seam(vec![ContentType::Code], Stub::new());
        cache.get("https://github.com/org/repo.git", None).unwrap();
        assert_eq!(*cache.seam.git_calls.borrow(), 1);
        assert_eq!(*cache.seam.path_calls.borrow(), 0);
        cache.get("/tmp/local", None).unwrap();
        assert_eq!(*cache.seam.path_calls.borrow(), 1);
    }

    #[test]
    fn cache_failure_not_poisoned() {
        let mut seam = Stub::new();
        seam.fail = true;
        let mut cache = IndexCache::with_seam(vec![ContentType::Code], seam);
        assert!(cache.get("/tmp/will-fail", None).is_err());
        assert_eq!(cache.size(), 0);
    }

    #[test]
    fn get_index_rejects_unsafe_schemes() {
        let mut cache = IndexCache::with_seam(vec![ContentType::Code], Stub::new());
        for url in [
            "ssh://git@github.com/o/r.git",
            "git://github.com/o/r.git",
            "file:///tmp/x",
        ] {
            let err = get_index(Some(url), None, None, &mut cache).unwrap_err();
            assert!(err.contains("Only https://, http://"), "{url}: {err}");
        }
    }

    #[test]
    fn get_index_requires_source() {
        let mut cache = IndexCache::with_seam(vec![ContentType::Code], Stub::new());
        let err = get_index(None, None, None, &mut cache).unwrap_err();
        assert!(err.contains("No repo specified"));
    }

    #[test]
    fn get_index_allows_https_and_path() {
        let mut cache = IndexCache::with_seam(vec![ContentType::Code], Stub::new());
        assert!(get_index(Some("https://github.com/o/r.git"), None, None, &mut cache).is_ok());
        assert!(get_index(None, Some("/tmp/default"), None, &mut cache).is_ok());
    }

    #[test]
    fn search_tool_no_results() {
        let mut cache = IndexCache::with_seam(vec![ContentType::Code], Stub::new());
        let out = search_tool(&mut cache, Some("/tmp/repo"), None, "anything", None, 5);
        assert_eq!(out, json!({ "error": "No results found." }).to_string());
    }

    struct OneChunkSeam;
    impl LoadOrBuild for OneChunkSeam {
        fn load_or_build(
            &self,
            _s: &str,
            _c: &[ContentType],
            _r: Option<&str>,
        ) -> Result<CspIndex, String> {
            Ok(index_with_chunk())
        }
    }

    #[test]
    fn search_tool_returns_results_json() {
        let mut cache = IndexCache::with_seam(vec![ContentType::Code], OneChunkSeam);
        let out = search_tool(&mut cache, Some("/tmp/repo"), None, "main", None, 5);
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(value.get("query").is_some());
        assert!(value["results"].as_array().is_some());
    }

    #[test]
    fn find_related_no_chunk_message() {
        let mut cache = IndexCache::with_seam(vec![ContentType::Code], OneChunkSeam);
        let out = find_related_tool(&mut cache, Some("/tmp/repo"), None, "nope.ts", 1, None, 5);
        assert!(out.contains("No chunk found at nope.ts:1"));
    }

    #[test]
    fn find_related_returns_json_for_known_chunk() {
        let mut cache = IndexCache::with_seam(vec![ContentType::Code], OneChunkSeam);
        let out = find_related_tool(&mut cache, Some("/tmp/repo"), None, "a.ts", 5, None, 5);
        // Either related results or the no-related error — both valid JSON.
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(value.get("query").is_some() || value.get("error").is_some());
    }
}
