use super::*;
use crate::indexing::cache::{resolve_cache_dir, CacheLocation};
use crate::indexing::dense::{make_stub_model, SelectableBasicBackend, DEFAULT_MODEL_NAME};
use crate::indexing::index::{CspIndexState, DEFAULT_CONTENT};
use crate::indexing::sparse::Bm25Index;
use crate::types::{Chunk, ContentType};
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
        files: Default::default(),
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
fn try_reuse_rejects_mismatched_model_kind() {
    let idx = build_index(vec![make_chunk("a.ts", "A")]);
    let dir = tempdir().unwrap();
    idx.save(dir.path(), Some("deadbeef")).unwrap();

    let manifest_path = dir.path().join("manifest.json");
    let raw = std::fs::read_to_string(&manifest_path).unwrap();
    let mut value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    value["modelKind"] = serde_json::json!("static");
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
    let manifest_path = cache_dir.join("manifest.json");
    assert!(manifest_path.exists());

    // Add an ignored sentinel to make reuse observable: a rebuild rewrites the
    // manifest from the typed struct and removes this extra field.
    let raw = std::fs::read_to_string(&manifest_path).unwrap();
    let mut manifest: serde_json::Value = serde_json::from_str(&raw).unwrap();
    manifest["reuseSentinel"] = serde_json::json!(true);
    std::fs::write(&manifest_path, manifest.to_string()).unwrap();

    // Hit: a second call reuses the cache without rewriting the manifest.
    let second = load_or_build_index(&src_str, &opts).unwrap();
    assert_eq!(second.chunks.len(), first.chunks.len());
    let after_hit: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&manifest_path).unwrap()).unwrap();
    assert_eq!(after_hit["reuseSentinel"], serde_json::json!(true));

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

// --- load_previous_for_incremental (mirrors upstream tests/index/test_create.py) ---

/// Build a real, well-formed cache for a two-file source and return
/// `(source dir, cache dir)`; `home` keeps the cache alive for the caller.
fn build_valid_cache(home: &Path) -> (tempfile::TempDir, PathBuf) {
    let src = tempdir().unwrap();
    std::fs::write(src.path().join("a.ts"), "function alpha() { return 1 }\n").unwrap();
    std::fs::write(src.path().join("b.ts"), "function beta() { return 2 }\n").unwrap();
    let src_str = src.path().to_string_lossy().into_owned();
    let base = home.join(".csp");
    let opts = LoadOrBuildOptions {
        base_dir: Some(base.clone()),
        ..Default::default()
    };
    load_or_build_index(&src_str, &opts).unwrap();
    let cache_dir = resolve_cache_dir(
        &src_str,
        DEFAULT_CONTENT,
        &CacheLocation {
            base_dir: Some(base),
            git_ref: None,
        },
    );
    (src, cache_dir)
}

fn read_json(path: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn write_json(path: &Path, value: &serde_json::Value) {
    std::fs::write(path, value.to_string()).unwrap();
}

#[test]
fn load_previous_for_incremental_happy_path() {
    let home = tempdir().unwrap();
    let (_src, cache_dir) = build_valid_cache(home.path());

    let previous =
        load_previous_for_incremental(&cache_dir, DEFAULT_MODEL_NAME, DEFAULT_CONTENT).unwrap();
    assert_eq!(previous.chunks.len(), previous.vectors.len());
    assert_eq!(previous.chunks.len(), previous.bm25_index.doc_order().len());
    assert!(previous.files.contains_key("a.ts"));
    assert!(previous.files.contains_key("b.ts"));
}

#[test]
fn load_previous_for_incremental_fails_closed() {
    for corrupt in [
        "missing_cache",
        "missing_files_key",
        "metadata_mismatch",
        "schema_version_mismatch",
        "component_length_mismatch",
        "length_mismatch",
        "overlapping_entries",
        "bm25_order_mismatch",
        "corrupt_json",
    ] {
        let home = tempdir().unwrap();
        let cache_dir = if corrupt == "missing_cache" {
            home.path().join("no-such-cache")
        } else {
            let (_src, cache_dir) = build_valid_cache(home.path());
            let manifest_path = cache_dir.join("manifest.json");
            let mut manifest = read_json(&manifest_path);
            match corrupt {
                "missing_files_key" => {
                    manifest.as_object_mut().unwrap().remove("files");
                }
                "metadata_mismatch" => manifest["modelId"] = serde_json::json!("other/model"),
                "schema_version_mismatch" => manifest["schemaVersion"] = serde_json::json!(1),
                "component_length_mismatch" => {
                    let chunks_path = cache_dir.join("chunks.json");
                    let mut chunks = read_json(&chunks_path);
                    chunks.as_array_mut().unwrap().pop();
                    write_json(&chunks_path, &chunks);
                }
                "length_mismatch" => {
                    let count = manifest["files"]["a.ts"]["count"].as_u64().unwrap();
                    manifest["files"]["a.ts"]["count"] = serde_json::json!(count + 5);
                }
                "overlapping_entries" => {
                    let start = manifest["files"]["a.ts"]["start"].clone();
                    manifest["files"]["b.ts"]["start"] = start;
                }
                "bm25_order_mismatch" => {
                    let bm25_path = cache_dir.join("bm25.json");
                    let mut bm25 = read_json(&bm25_path);
                    bm25["docOrder"].as_array_mut().unwrap().reverse();
                    write_json(&bm25_path, &bm25);
                }
                "corrupt_json" => {
                    std::fs::write(&manifest_path, "{not json").unwrap();
                }
                _ => unreachable!(),
            }
            if corrupt != "corrupt_json" {
                write_json(&manifest_path, &manifest);
            }
            cache_dir
        };

        assert!(
            load_previous_for_incremental(&cache_dir, DEFAULT_MODEL_NAME, DEFAULT_CONTENT)
                .is_none(),
            "case {corrupt} should fail closed"
        );
    }
}

#[test]
fn load_previous_for_incremental_compares_content_as_a_set() {
    let home = tempdir().unwrap();
    let (_src, cache_dir) = build_valid_cache(home.path());
    let manifest_path = cache_dir.join("manifest.json");
    let mut manifest = read_json(&manifest_path);
    let duplicated = [ContentType::Code, ContentType::Code];

    // Same length, and every requested type is present in the manifest — but
    // the manifest also covers docs the request does not ask for.
    manifest["content"] = serde_json::json!(["code", "docs"]);
    write_json(&manifest_path, &manifest);
    assert!(load_previous_for_incremental(&cache_dir, DEFAULT_MODEL_NAME, &duplicated).is_none());

    // Repetition on the request side is irrelevant once the sets agree.
    manifest["content"] = serde_json::json!(["code"]);
    write_json(&manifest_path, &manifest);
    assert!(load_previous_for_incremental(&cache_dir, DEFAULT_MODEL_NAME, &duplicated).is_some());
}

#[test]
fn load_or_build_reuses_unchanged_files_on_incremental_rebuild() {
    let home = tempdir().unwrap();
    let (src, cache_dir) = build_valid_cache(home.path());
    let src_str = src.path().to_string_lossy().into_owned();
    let opts = LoadOrBuildOptions {
        base_dir: Some(home.path().join(".csp")),
        ..Default::default()
    };

    // Plant a sentinel in the cached chunk for the file that will not change: it
    // can only reach the rebuilt index by being reused from the previous index.
    let chunks_path = cache_dir.join("chunks.json");
    let mut chunks = read_json(&chunks_path);
    let a_chunk = chunks
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|c| c["filePath"] == "a.ts")
        .unwrap();
    let content = format!("{}/*reused*/", a_chunk["content"].as_str().unwrap());
    a_chunk["content"] = serde_json::json!(content);
    write_json(&chunks_path, &chunks);
    let b_hash_before =
        read_json(&cache_dir.join("manifest.json"))["files"]["b.ts"]["hash"].clone();

    // Change b.ts → whole-tree hash mismatch → incremental rebuild.
    std::fs::write(src.path().join("b.ts"), "function beta() { return 22 }\n").unwrap();
    let rebuilt = load_or_build_index(&src_str, &opts).unwrap();

    let a_chunk = rebuilt
        .chunks
        .iter()
        .find(|c| c.file_path == "a.ts")
        .unwrap();
    assert!(a_chunk.content.ends_with("/*reused*/"));
    let b_chunk = rebuilt
        .chunks
        .iter()
        .find(|c| c.file_path == "b.ts")
        .unwrap();
    assert!(b_chunk.content.contains("22"));

    // The persisted manifest now records b.ts's new hash and is a valid seed.
    let manifest = read_json(&cache_dir.join("manifest.json"));
    assert_ne!(manifest["files"]["b.ts"]["hash"], b_hash_before);
    assert!(
        load_previous_for_incremental(&cache_dir, DEFAULT_MODEL_NAME, DEFAULT_CONTENT).is_some()
    );
}
