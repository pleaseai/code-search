use super::*;

fn chunk(file_path: &str, content: &str) -> Chunk {
    Chunk {
        content: content.to_string(),
        file_path: file_path.to_string(),
        start_line: 1,
        end_line: 1,
        language: None,
    }
}

fn docs(input: &[&[&str]]) -> Vec<Vec<String>> {
    input
        .iter()
        .map(|d| d.iter().map(|s| s.to_string()).collect())
        .collect()
}

fn query(tokens: &[&str]) -> Vec<String> {
    tokens.iter().map(|s| s.to_string()).collect()
}

// --- enrich_for_bm25 (mirrors src/indexing/sparse.test.ts) ---

#[test]
fn enrich_appends_repeated_stem_and_dir_parts() {
    assert_eq!(
        enrich_for_bm25(&chunk("src/utils/format.ts", "hello world")),
        "hello world format format src utils"
    );
}

#[test]
fn enrich_trims_to_last_3_dir_parts() {
    assert_eq!(
        enrich_for_bm25(&chunk("a/b/c/d/foo.py", "x")),
        "x foo foo b c d"
    );
}

#[test]
fn enrich_handles_top_level_file() {
    assert_eq!(enrich_for_bm25(&chunk("foo.py", "x")), "x foo foo ");
}

#[test]
fn enrich_drops_dot_segments() {
    assert_eq!(
        enrich_for_bm25(&chunk("./a/b/foo.ts", "x")),
        "x foo foo a b"
    );
}

#[test]
fn enrich_normalizes_backslashes() {
    assert_eq!(
        enrich_for_bm25(&chunk("src\\utils\\format.ts", "hello world")),
        "hello world format format src utils"
    );
}

// --- selector_to_mask ---

#[test]
fn selector_builds_mask() {
    let mask = selector_to_mask(Some(&[0, 2, 5]), 6).unwrap();
    assert_eq!(mask, vec![1, 0, 1, 0, 0, 1]);
}

#[test]
fn selector_none_returns_none() {
    assert_eq!(selector_to_mask(None, 6), None);
}

#[test]
fn selector_ignores_out_of_bounds() {
    let mask = selector_to_mask(Some(&[0, 10]), 3).unwrap();
    assert_eq!(mask, vec![1, 0, 0]);
}

// --- Bm25Index ---

#[test]
fn ranks_docs_with_query_term_higher() {
    let index = Bm25Index::build(&docs(&[&["hello", "world"], &["hello"], &["world"]]));
    let scores = index.get_scores(&query(&["hello"]), None);
    assert_eq!(scores.len(), 3);
    assert!(scores[0] > 0.0);
    assert!(scores[1] > 0.0);
    assert_eq!(scores[2], 0.0);
}

#[test]
fn zero_scores_for_unknown_tokens() {
    let index = Bm25Index::build(&docs(&[&["hello"], &["world"]]));
    assert_eq!(index.get_scores(&query(&["unknown"]), None), vec![0.0, 0.0]);
}

#[test]
fn empty_corpus_yields_empty_scores() {
    let index = Bm25Index::build(&docs(&[]));
    assert_eq!(index.get_scores(&query(&["anything"]), None).len(), 0);
}

#[test]
fn empty_query_yields_zero_scores() {
    let index = Bm25Index::build(&docs(&[&["hello"], &["world"]]));
    assert_eq!(index.get_scores(&[], None), vec![0.0, 0.0]);
}

#[test]
fn weight_mask_zeros_masked_docs() {
    let index = Bm25Index::build(&docs(&[&["hello", "world"], &["hello"], &["world"]]));
    let scores = index.get_scores(&query(&["hello"]), Some(&[1, 0, 1]));
    assert!(scores[0] > 0.0);
    assert_eq!(scores[1], 0.0);
    assert_eq!(scores[2], 0.0);
}

#[test]
fn full_mask_matches_baseline() {
    let index = Bm25Index::build(&docs(&[&["hello", "world"], &["hello"], &["world"]]));
    let baseline = index.get_scores(&query(&["hello"]), None);
    let masked = index.get_scores(&query(&["hello"]), Some(&[1, 1, 1]));
    assert_eq!(masked, baseline);
}

#[test]
fn repeated_query_tokens_do_not_compound() {
    let index = Bm25Index::build(&docs(&[&["hello"]]));
    let single = index.get_scores(&query(&["hello"]), None);
    let repeated = index.get_scores(&query(&["hello", "hello", "hello"]), None);
    assert_eq!(repeated, single);
}

// --- save / load (T014) ---

#[test]
fn save_load_round_trips_scores() {
    let index = Bm25Index::build(&docs(&[
        &["hello", "world"],
        &["hello"],
        &["world", "world"],
    ]));
    let dir = tempfile::tempdir().unwrap();
    index.save(dir.path()).unwrap();

    let loaded = Bm25Index::load(dir.path()).unwrap();
    assert_eq!(loaded.num_docs(), index.num_docs());
    for q in [
        query(&["hello"]),
        query(&["world"]),
        query(&["hello", "world"]),
    ] {
        assert_eq!(loaded.get_scores(&q, None), index.get_scores(&q, None));
    }
}

#[test]
fn save_writes_documents_and_doc_order() {
    let index = Bm25Index::build(&docs(&[&["hello", "hello", "world"]]));
    let dir = tempfile::tempdir().unwrap();
    index.save(dir.path()).unwrap();

    let raw = std::fs::read_to_string(dir.path().join("bm25.json")).unwrap();
    let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(value["version"], 2);
    assert_eq!(value["documents"]["0"]["hello"], 2);
    assert_eq!(value["documents"]["0"]["world"], 1);
    assert_eq!(value["docOrder"], serde_json::json!(["0"]));
}

#[test]
fn load_missing_file_is_err() {
    let dir = tempfile::tempdir().unwrap();
    assert!(Bm25Index::load(dir.path()).is_err());
}

// --- incremental API (mirrors upstream tests/index/test_bm25.py) ---

fn build_ids(input: &[(&str, &[&str])]) -> Bm25Index {
    let mut index = Bm25Index::new();
    for (chunk_id, tokens) in input {
        index.add_document(chunk_id, &query(tokens)).unwrap();
    }
    index.set_doc_order(input.iter().map(|(id, _)| id.to_string()).collect());
    index
}

#[test]
fn scoring_matches_lucene_formula() {
    let index = build_ids(&[
        ("a", &["authenticate", "token"]),
        ("b", &["login", "password"]),
    ]);
    let scores = index.get_scores(&query(&["authenticate"]), None);
    // idf = ln(1 + 1.5/1.5); tf term = 1·(k1+1) / (1 + k1·(1 − b + b·dl/avgdl)) = 2.5/2.5.
    let expected = (1.0f64 + 1.5 / 1.5).ln() as f32;
    assert!(
        (scores[0] - expected).abs() < 1e-6,
        "{} vs {expected}",
        scores[0]
    );
    assert_eq!(scores[1], 0.0);
}

#[test]
fn removed_and_unordered_documents_stop_scoring() {
    let mut index = build_ids(&[("a", &["authenticate"]), ("b", &["login"])]);
    index.remove_document("missing");
    index.set_doc_order(vec!["b".to_string()]);
    assert_eq!(index.get_scores(&query(&["authenticate"]), None), vec![0.0]);

    index.remove_document("a");
    index.set_doc_order(vec!["a".to_string(), "b".to_string()]);
    assert_eq!(
        index.get_scores(&query(&["authenticate"]), None),
        vec![0.0, 0.0]
    );
    assert_eq!(index.corpus_size(), 1);
    assert_eq!(index.num_docs(), 2);
}

#[test]
fn duplicate_add_document_errors() {
    let mut index = build_ids(&[("a", &["x"])]);
    let err = index.add_document("a", &query(&["y"])).unwrap_err();
    assert!(err.contains("already indexed"));
}

#[test]
fn removed_slot_is_recycled_without_stale_postings() {
    let mut index = build_ids(&[("a", &["alpha"]), ("b", &["beta"])]);
    index.remove_document("a");
    index.add_document("c", &query(&["gamma"])).unwrap();
    index.set_doc_order(vec!["b".to_string(), "c".to_string()]);
    assert_eq!(index.get_scores(&query(&["alpha"]), None), vec![0.0, 0.0]);
    let gamma = index.get_scores(&query(&["gamma"]), None);
    assert_eq!(gamma[0], 0.0);
    assert!(gamma[1] > 0.0);
}

#[test]
fn save_load_preserves_scores_and_doc_order() {
    let index = build_ids(&[
        ("empty", &[]),
        ("a", &["authenticate", "token"]),
        ("b", &["login", "password"]),
    ]);
    let dir = tempfile::tempdir().unwrap();
    index.save(dir.path()).unwrap();

    let loaded = Bm25Index::load(dir.path()).unwrap();
    assert_eq!(loaded.doc_order(), index.doc_order());
    assert_eq!(
        loaded.get_scores(&query(&["authenticate"]), None),
        index.get_scores(&query(&["authenticate"]), None)
    );
}

#[test]
fn load_rejects_zero_term_frequency() {
    let index = build_ids(&[("a", &["authenticate"])]);
    let dir = tempfile::tempdir().unwrap();
    index.save(dir.path()).unwrap();
    let path = dir.path().join("bm25.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    value["documents"]["a"]["authenticate"] = serde_json::json!(0);
    std::fs::write(&path, value.to_string()).unwrap();

    let err = Bm25Index::load(dir.path()).unwrap_err();
    assert!(err.to_string().contains("must be positive"));
}

#[test]
fn load_rejects_inconsistent_document_order() {
    let index = build_ids(&[("a", &["authenticate"])]);
    let dir = tempfile::tempdir().unwrap();
    index.save(dir.path()).unwrap();
    let path = dir.path().join("bm25.json");
    let mut value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    value["docOrder"] = serde_json::json!(["other"]);
    std::fs::write(&path, value.to_string()).unwrap();

    let err = Bm25Index::load(dir.path()).unwrap_err();
    assert!(err.to_string().contains("document state"));
}
