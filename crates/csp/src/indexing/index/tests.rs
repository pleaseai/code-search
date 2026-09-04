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
        files: Default::default(),
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
    assert_eq!(value["schemaVersion"], 2);
    assert_eq!(value["modelId"], "test-model");
    assert_eq!(value["content"], serde_json::json!(["code"]));
    assert!(value["contentHash"].as_str().unwrap().len() == 64);
    assert_eq!(
        value["chunkSize"].as_u64(),
        Some(u64::from(DESIRED_CHUNK_LENGTH_CHARS as u32))
    );
    assert!(value["files"].is_object());
}

#[test]
fn save_load_roundtrip_preserves_file_manifest() {
    let src = tempdir().unwrap();
    std::fs::write(src.path().join("a.ts"), "export const alpha = 1\n").unwrap();
    let idx = CspIndex::from_path(src.path(), &LoadOptions::default()).unwrap();
    assert!(idx.files.contains_key("a.ts"));

    let dir = tempdir().unwrap();
    idx.save(dir.path(), None).unwrap();
    let loaded = CspIndex::load_from_disk(dir.path()).unwrap();
    assert_eq!(loaded.files, idx.files);
    assert_eq!(loaded.bm25_index.doc_order(), idx.bm25_index.doc_order());
}

#[test]
fn load_rejects_inconsistent_component_counts() {
    let idx = build_index(vec![
        make_chunk("a.ts", 1, 10, Some("typescript"), "A"),
        make_chunk("b.ts", 1, 5, Some("python"), "B"),
    ]);
    let dir = tempdir().unwrap();
    idx.save(dir.path(), None).unwrap();
    let chunks_path = dir.path().join("chunks.json");
    let mut chunks: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(&chunks_path).unwrap()).unwrap();
    chunks.pop();
    std::fs::write(&chunks_path, serde_json::to_string(&chunks).unwrap()).unwrap();

    let err = CspIndex::load_from_disk(dir.path()).unwrap_err();
    assert!(err.contains("inconsistent document counts"));
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
    let ha: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir_a.path().join("manifest.json")).unwrap())
            .unwrap();
    let hb: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir_b.path().join("manifest.json")).unwrap())
            .unwrap();
    assert_eq!(ha["contentHash"], hb["contentHash"]);
}

#[test]
fn from_path_errors_on_missing() {
    let dir = tempdir().unwrap();
    let err = CspIndex::from_path(&dir.path().join("nope"), &LoadOptions::default()).unwrap_err();
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

#[test]
fn from_path_stores_absolute_root_for_relative_path() {
    // Built from a cwd-relative path; `root` must still come out absolute so a
    // reload from another cwd can find the source tree (upstream `path.resolve()`).
    let dir = tempfile::Builder::new().tempdir_in(".").unwrap();
    std::fs::write(dir.path().join("sample.ts"), "export const x = 1\n").unwrap();
    let relative = Path::new(dir.path().file_name().unwrap());
    assert!(relative.is_relative());
    let idx = CspIndex::from_path(relative, &LoadOptions::default()).unwrap();
    let root = idx.root.as_deref().unwrap();
    assert!(Path::new(root).is_absolute(), "root was {root}");
    assert!(root.ends_with(relative.to_str().unwrap()));
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
            .ok()
    };
    let Some(output) = run(&["init", "-q"]) else {
        return; // git unavailable — skip rather than fail.
    };
    if !output.status.success() {
        return;
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

#[test]
fn compute_file_sizes_skips_paths_that_escape_root() {
    let outer = tempdir().unwrap();
    let root = outer.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("inside.ts"), "abc").unwrap();
    std::fs::write(outer.path().join("secret.txt"), "top secret").unwrap();
    let abs = root.join("inside.ts").to_string_lossy().into_owned();

    let chunks = vec![
        make_chunk("inside.ts", 1, 1, None, "abc"),
        make_chunk("../secret.txt", 1, 1, None, "x"),
        make_chunk(&abs, 1, 1, None, "x"),
    ];
    let sizes = compute_file_sizes(&root, &chunks);

    assert_eq!(sizes.get("inside.ts"), Some(&3));
    assert!(!sizes.contains_key("../secret.txt"));
    assert!(!sizes.contains_key(abs.as_str()));
}

#[cfg(unix)]
#[test]
fn compute_file_sizes_skips_symlinks_and_non_regular_files() {
    let outer = tempdir().unwrap();
    let root = outer.path().join("repo");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("real.ts"), "abcd").unwrap();
    std::fs::write(outer.path().join("secret.txt"), "top secret").unwrap();
    std::os::unix::fs::symlink(outer.path().join("secret.txt"), root.join("link.ts")).unwrap();
    std::fs::create_dir(root.join("dir.ts")).unwrap();

    let chunks = vec![
        make_chunk("real.ts", 1, 1, None, "abcd"),
        make_chunk("link.ts", 1, 1, None, "x"),
        make_chunk("dir.ts", 1, 1, None, "x"),
    ];
    let sizes = compute_file_sizes(&root, &chunks);

    assert_eq!(sizes.get("real.ts"), Some(&4));
    assert!(!sizes.contains_key("link.ts"));
    assert!(!sizes.contains_key("dir.ts"));
}

#[test]
fn from_path_reads_file_sizes_lazily() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("sample.ts"), "export const x = 1\n").unwrap();

    let idx = CspIndex::from_path(dir.path(), &LoadOptions::default()).unwrap();

    assert!(idx.file_sizes.is_available());
    // Look it up under the path the chunks carry, so a chunk-path/root drift
    // (absolute paths, a stray prefix) fails here instead of silently zeroing
    // `file_chars` at telemetry time.
    assert_eq!(idx.chunks[0].file_path, "sample.ts");
    assert_eq!(idx.file_sizes.get(&idx.chunks[0].file_path), Some(19));
}

#[test]
fn load_from_disk_has_no_file_sizes_when_source_is_gone() {
    let source = tempdir().unwrap();
    std::fs::write(source.path().join("sample.ts"), "export const x = 1\n").unwrap();
    let idx = CspIndex::from_path(source.path(), &LoadOptions::default()).unwrap();
    let cache = tempdir().unwrap();
    idx.save(cache.path(), None).unwrap();
    std::fs::remove_dir_all(source.path()).unwrap();

    let loaded = CspIndex::load_from_disk(cache.path()).unwrap();

    assert!(!loaded.file_sizes.is_available());
    assert_eq!(loaded.file_sizes.get("sample.ts"), None);
}
