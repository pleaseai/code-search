use super::*;
use crate::indexing::cache::{resolve_cache_dir, CacheLocation};
use crate::indexing::dense::{make_stub_model, SelectableBasicBackend};
use crate::indexing::index::{CspIndexState, DEFAULT_CONTENT};
use crate::indexing::sparse::Bm25Index;
use crate::types::Chunk;
use tempfile::tempdir;

fn make_chunk(file_path: &str, content: &str) -> Chunk {
    Chunk {
        content: content.to_string(),
        file_path: file_path.to_string(),
        start_line: 1,
        end_line: 10,
        language: Some("typescript".to_string()),
    }
}

fn build_index(chunks: Vec<Chunk>) -> CspIndex {
    let model = make_stub_model(4);
    let vectors = vec![vec![1.0, 0.0, 0.0, 0.0]; chunks.len()];
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
fn hash_path_normalizes_platform_separators() {
    assert_eq!(normalized_hash_path(Path::new(r"src\lib.rs")), "src/lib.rs");
    assert_eq!(normalized_hash_path(Path::new("src/lib.rs")), "src/lib.rs");
}

#[test]
fn try_reuse_rejects_stale_chunk_size() {
    let chunks = vec![make_chunk("a.ts", "A")];
    let idx = build_index(chunks);
    let dir = tempdir().unwrap();
    idx.save(dir.path(), Some("deadbeef")).unwrap();

    // Fresh cache (matching hash + current chunk_size) is reused.
    assert!(try_reuse(dir.path(), false, Some("deadbeef"), "test-model").is_some());

    // Rewrite the manifest with a different chunk_size → stale → rebuild.
    let manifest_path = dir.path().join("manifest.json");
    let raw = std::fs::read_to_string(&manifest_path).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    value["chunkSize"] = serde_json::json!(9999);
    std::fs::write(&manifest_path, value.to_string()).unwrap();
    assert!(try_reuse(dir.path(), false, Some("deadbeef"), "test-model").is_none());

    // A manifest predating the field (absent chunkSize) is also stale.
    let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    value.as_object_mut().unwrap().remove("chunkSize");
    std::fs::write(&manifest_path, value.to_string()).unwrap();
    assert!(try_reuse(dir.path(), false, Some("deadbeef"), "test-model").is_none());
}

#[test]
fn try_reuse_rejects_stale_model() {
    let chunks = vec![make_chunk("a.ts", "A")];
    let idx = build_index(chunks);
    let dir = tempdir().unwrap();
    idx.save(dir.path(), Some("deadbeef")).unwrap();

    // Same model → reused; a different model (e.g. after a default bump to
    // `-v2`) → stale even with a matching hash + chunk_size.
    assert!(try_reuse(dir.path(), false, Some("deadbeef"), "test-model").is_some());
    assert!(try_reuse(dir.path(), false, Some("deadbeef"), "other-model").is_none());
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
