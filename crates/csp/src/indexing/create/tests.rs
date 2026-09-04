use super::*;
use crate::indexing::dense::make_stub_model;
use crate::tokens::tokenize;
use tempfile::tempdir;

fn opts(model: &Model, display_root: Option<PathBuf>) -> CreateIndexOptions<'_> {
    CreateIndexOptions {
        display_root,
        ..CreateIndexOptions::new(model)
    }
}

#[test]
fn builds_indexes_for_small_ts_file() {
    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("sample.ts"),
        "export function greet(name: string) {\n  return `hi ${name}`\n}\n",
    )
    .unwrap();
    let model = make_stub_model(4);
    let result = create_index_from_path(
        dir.path(),
        &opts(&model, Some(dir.path().to_path_buf())),
        None,
    )
    .unwrap();

    assert!(!result.chunks.is_empty());
    assert_eq!(result.chunks[0].file_path, "sample.ts");
    assert_eq!(result.semantic_index.vectors.len(), result.chunks.len());
    assert_eq!(result.bm25_index.num_docs(), result.chunks.len());
}

#[test]
fn errors_when_no_supported_files() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("data.bin"), "binary").unwrap();
    let model = make_stub_model(4);
    let err = create_index_from_path(dir.path(), &opts(&model, None), None).unwrap_err();
    assert!(err.contains("No supported files found"));
}

#[test]
fn respects_extensions_override() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hello world").unwrap();
    let model = make_stub_model(4);
    let options = CreateIndexOptions {
        model: &model,
        extensions: Some(vec![".txt".to_string()]),
        content: Some(vec![ContentType::Docs]),
        display_root: Some(dir.path().to_path_buf()),
        max_file_bytes: None,
    };
    let result = create_index_from_path(dir.path(), &options, None).unwrap();
    assert_eq!(result.chunks.len(), 1);
    assert_eq!(result.chunks[0].file_path, "a.txt");
}

#[test]
fn skips_files_over_max_bytes() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("big.ts"), "a".repeat(2_000_000)).unwrap();
    std::fs::write(dir.path().join("small.ts"), "export const x = 1\n").unwrap();
    let model = make_stub_model(4);
    let result = create_index_from_path(
        dir.path(),
        &opts(&model, Some(dir.path().to_path_buf())),
        None,
    )
    .unwrap();
    let paths: Vec<&str> = result.chunks.iter().map(|c| c.file_path.as_str()).collect();
    assert!(paths.contains(&"small.ts"));
    assert!(!paths.contains(&"big.ts"));
}

#[test]
fn honors_configured_max_file_bytes() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("mid.ts"), "a".repeat(200)).unwrap();
    std::fs::write(dir.path().join("small.ts"), "export const x = 1\n").unwrap();
    let model = make_stub_model(4);
    let indexed = |limit: u64| -> Vec<String> {
        let options = CreateIndexOptions {
            max_file_bytes: Some(limit),
            ..opts(&model, Some(dir.path().to_path_buf()))
        };
        let result = create_index_from_path(dir.path(), &options, None).unwrap();
        result.chunks.iter().map(|c| c.file_path.clone()).collect()
    };

    // A limit under the default still gates: 200 bytes > 100.
    let paths = indexed(100);
    assert!(paths.iter().any(|p| p == "small.ts"));
    assert!(!paths.iter().any(|p| p == "mid.ts"));
    // Raising it lets the same file through.
    assert!(indexed(1_000).iter().any(|p| p == "mid.ts"));
}

#[test]
fn skipped_large_warning_names_first_five_paths_and_count() {
    assert_eq!(skipped_large_warning(&[], 1_000_000, true), None);

    let two = vec!["a.ts".to_string(), "b.ts".to_string()];
    let msg = skipped_large_warning(&two, 1_000_000, true).unwrap();
    assert_eq!(
        msg,
        "Skipped 2 file(s) exceeding the maximum file size of 1000000 bytes \
         (raise CSP_MAX_FILE_BYTES to include them): a.ts, b.ts"
    );

    let seven: Vec<String> = (1..=7).map(|i| format!("f{i}.ts")).collect();
    let msg = skipped_large_warning(&seven, 500, true).unwrap();
    assert!(msg.starts_with("Skipped 7 file(s) exceeding the maximum file size of 500 bytes"));
    assert!(
        msg.ends_with("f1.ts, f2.ts, f3.ts, f4.ts, f5.ts ..."),
        "{msg}"
    );
    assert!(!msg.contains("f6.ts"));

    // A caller-pinned limit points at the option, not the env var.
    let msg = skipped_large_warning(&two, 100, false).unwrap();
    assert!(
        msg.contains("(raise max_file_bytes to include them)"),
        "{msg}"
    );
    assert!(!msg.contains("CSP_MAX_FILE_BYTES"));
}

#[test]
fn skipped_large_warning_escapes_control_characters_in_paths() {
    let hostile = vec!["evil\x1b[31m\nfake: ok.ts".to_string()];
    let msg = skipped_large_warning(&hostile, 10, true).unwrap();
    assert!(msg.ends_with("evil\\u{1b}[31m\\nfake: ok.ts"), "{msg}");
    assert!(!msg.contains('\n'));
}

#[test]
fn descends_into_subdirectories() {
    let dir = tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/nested.ts"), "const a = 1\n").unwrap();
    let model = make_stub_model(4);
    let result = create_index_from_path(
        dir.path(),
        &opts(&model, Some(dir.path().to_path_buf())),
        None,
    )
    .unwrap();
    assert!(result
        .chunks
        .iter()
        .any(|c| c.file_path.ends_with("nested.ts")));
}

// --- incremental reindex (mirrors upstream tests/index/test_create.py) ---

fn write_files(root: &Path, files: &[(&str, &str)]) {
    for (rel, content) in files {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }
}

fn into_previous(result: CreateIndexResult) -> PreviousIndex {
    PreviousIndex::try_new(
        result.chunks,
        result.semantic_index.vectors,
        result.files,
        result.bm25_index,
    )
    .unwrap()
}

fn score_sum(index: &Bm25Index, query: &str) -> f32 {
    index.get_scores(&tokenize(query), None).iter().sum()
}

#[test]
fn fresh_build_records_a_layout_valid_file_manifest() {
    let dir = tempdir().unwrap();
    write_files(
        dir.path(),
        &[
            ("a.ts", "function stable_anchor() { return 1 }\n"),
            ("sub/b.ts", "function other_value() { return 2 }\n"),
        ],
    );
    let model = make_stub_model(4);
    let result = create_index_from_path(
        dir.path(),
        &opts(&model, Some(dir.path().to_path_buf())),
        None,
    )
    .unwrap();

    assert_eq!(result.files.len(), 2);
    let total: usize = result.files.values().map(|e| e.count).sum();
    assert_eq!(total, result.chunks.len());
    for (path, entry) in &result.files {
        assert_eq!(entry.hash.len(), 64);
        assert!(result.chunks[entry.start..entry.end()]
            .iter()
            .all(|c| c.file_path == *path));
    }
    // The manifest, chunks, vectors, and BM25 order all agree.
    assert!(into_previous(result).files.contains_key("a.ts"));
}

#[test]
fn incremental_reindex_reuses_updates_and_prunes() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    write_files(
        &root,
        &[
            ("a.ts", "function stable_anchor() { return 1 }\n"),
            ("b.ts", "function changed_value() { return 2 }\n"),
            ("c.ts", "function unique_gone() { return 3 }\n"),
            ("emptying.ts", "function becomes_empty() { return 4 }\n"),
        ],
    );
    let model = make_stub_model(4);
    let before = create_index_from_path(&root, &opts(&model, Some(root.clone())), None).unwrap();
    let b_before = before.files["b.ts"].clone();
    let b_vectors_before = before.semantic_index.vectors[b_before.start..b_before.end()].to_vec();
    let a_before = before.files["a.ts"].clone();

    // Plant sentinels in the previous index for the unchanged file: they can
    // only survive into the rebuilt index if its rows were reused, not
    // re-chunked/re-embedded (the stub embedder is deterministic).
    let mut previous = into_previous(before);
    let sentinel_vector = vec![0.0, 1.0, 0.0, 0.0];
    previous.vectors[a_before.start] = sentinel_vector.clone();
    previous.chunks[a_before.start]
        .content
        .push_str("/*reused*/");

    write_files(
        &root,
        &[("b.ts", "function changed_value() { return 999 }\n")],
    );
    std::fs::remove_file(root.join("c.ts")).unwrap();
    write_files(&root, &[("emptying.ts", &" ".repeat(128))]);
    write_files(
        &root,
        &[("d.ts", "function brand_new_term() { return 4 }\n")],
    );

    let after =
        create_index_from_path(&root, &opts(&model, Some(root.clone())), Some(previous)).unwrap();

    let a_after = &after.files["a.ts"];
    assert_eq!(after.semantic_index.vectors[a_after.start], sentinel_vector);
    assert!(after.chunks[a_after.start].content.ends_with("/*reused*/"));
    let b_after = &after.files["b.ts"];
    assert_ne!(
        after.semantic_index.vectors[b_after.start..b_after.end()].to_vec(),
        b_vectors_before
    );
    assert!(!after.files.contains_key("c.ts"));
    assert!(after.files.contains_key("d.ts"));
    assert_eq!(after.files["emptying.ts"].count, 0);

    assert_eq!(score_sum(&after.bm25_index, "unique_gone"), 0.0);
    assert_eq!(score_sum(&after.bm25_index, "becomes_empty"), 0.0);
    assert!(score_sum(&after.bm25_index, "brand_new_term") > 0.0);
    assert!(score_sum(&after.bm25_index, "changed_value") > 0.0);

    let mut expected_ids: Vec<String> = after
        .files
        .iter()
        .flat_map(|(path, entry)| (0..entry.count).map(move |slot| make_chunk_id(path, slot)))
        .collect();
    expected_ids.sort();
    let mut doc_order = after.bm25_index.doc_order().to_vec();
    doc_order.sort();
    assert_eq!(doc_order, expected_ids);
    assert_eq!(after.bm25_index.corpus_size(), after.chunks.len());
    assert_eq!(after.semantic_index.vectors.len(), after.chunks.len());
    // The rebuilt index is itself a valid seed for the next incremental pass.
    into_previous(after);
}

#[test]
fn zero_chunk_file_does_not_break_manifest_tiling() {
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    // `pkg/` is walked before `pkg.ts` (directory entries sort by file name),
    // but "pkg.ts" < "pkg/z.ts" lexicographically ('.' 0x2E < '/' 0x2F). The
    // empty file yields no chunks, so both entries share `start`.
    write_files(
        &root,
        &[
            ("pkg/z.ts", "   \n"),
            ("pkg.ts", "function stable_anchor() { return 1 }\n"),
        ],
    );
    let model = make_stub_model(4);
    let result = create_index_from_path(&root, &opts(&model, Some(root.clone())), None).unwrap();
    // `display_path` keeps the platform separator, so build the key with `join`.
    let zero_path = Path::new("pkg").join("z.ts").to_string_lossy().into_owned();
    assert_eq!(result.files[zero_path.as_str()].count, 0);
    assert_eq!(
        result.files[zero_path.as_str()].start,
        result.files["pkg.ts"].start
    );
    // A freshly built index must always be a valid seed for the next pass.
    into_previous(result);
}

/// Non-UTF-8 file names only exist on Unix filesystems that allow them
/// (APFS rejects them), so this runs on Linux only.
#[cfg(target_os = "linux")]
#[test]
fn colliding_lossy_paths_skip_the_duplicate_instead_of_aborting() {
    use std::os::unix::ffi::OsStrExt;
    let dir = tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let first = root.join(std::ffi::OsStr::from_bytes(b"a\xff.ts"));
    let second = root.join(std::ffi::OsStr::from_bytes(b"a\xfe.ts"));
    std::fs::write(&first, "function first_file() { return 1 }\n").unwrap();
    std::fs::write(&second, "function second_file() { return 2 }\n").unwrap();
    assert_eq!(
        first.to_string_lossy(),
        second.to_string_lossy(),
        "test precondition: both names must collapse to one display path"
    );

    let model = make_stub_model(4);
    let result = create_index_from_path(&root, &opts(&model, Some(root.clone())), None).unwrap();
    assert_eq!(result.files.len(), 1);
    assert_eq!(result.bm25_index.corpus_size(), result.chunks.len());
    into_previous(result);
}
