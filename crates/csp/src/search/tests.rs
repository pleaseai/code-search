use super::*;
use std::cell::RefCell;

fn make_chunk(content: &str, file_path: &str, start_line: u32, end_line: u32) -> Chunk {
    Chunk {
        content: content.to_string(),
        file_path: file_path.to_string(),
        start_line,
        end_line,
        language: Some("ts".to_string()),
    }
}

fn make_chunks() -> Vec<Chunk> {
    vec![
        make_chunk("class Alpha {}", "src/alpha.ts", 10, 20),
        make_chunk("function beta() {}", "src/alpha.ts", 30, 40),
        make_chunk("export const gamma = 1", "src/gamma.ts", 1, 5),
        make_chunk("function delta() {}", "src/delta.ts", 5, 15),
        make_chunk("class Epsilon {}", "src/epsilon.ts", 50, 60),
    ]
}

struct MockModel;
impl EmbeddingModel for MockModel {
    fn encode(&self, texts: &[String]) -> Vec<Vec<f32>> {
        texts.iter().map(|_| vec![0.1, 0.2, 0.3]).collect()
    }
}

#[derive(Default)]
struct QueryCall {
    k: usize,
    selector: Option<Vec<u32>>,
}

struct MockSemantic {
    results: Vec<(usize, f64)>,
    calls: RefCell<Vec<QueryCall>>,
}
impl MockSemantic {
    fn new(results: Vec<(usize, f64)>) -> Self {
        Self {
            results,
            calls: RefCell::new(Vec::new()),
        }
    }
}
impl VectorBackend for MockSemantic {
    fn query(
        &self,
        _vectors: &[Vec<f32>],
        k: usize,
        selector: Option<&[u32]>,
    ) -> Vec<Vec<(usize, f64)>> {
        self.calls.borrow_mut().push(QueryCall {
            k,
            selector: selector.map(<[u32]>::to_vec),
        });
        vec![self.results.clone()]
    }
}

struct Bm25Call {
    mask: Option<Vec<u8>>,
}
struct MockBm25 {
    scores: Vec<f32>,
    calls: RefCell<Vec<Bm25Call>>,
}
impl MockBm25 {
    fn new(scores: Vec<f32>) -> Self {
        Self {
            scores,
            calls: RefCell::new(Vec::new()),
        }
    }
}
impl SparseBackend for MockBm25 {
    fn get_scores(&self, _tokens: &[String], weight_mask: Option<&[u8]>) -> Vec<f32> {
        self.calls.borrow_mut().push(Bm25Call {
            mask: weight_mask.map(<[u8]>::to_vec),
        });
        self.scores.clone()
    }
}

fn opts(alpha: Option<f64>, rerank: Option<bool>) -> SearchOptions {
    SearchOptions {
        alpha,
        selector: None,
        rerank,
    }
}

// --- sort_top_k ---

#[test]
fn sort_top_k_descending() {
    let out = sort_top_k(&[0.1, 0.9, 0.5, 0.3, 0.7], 3);
    assert_eq!(out, [1, 4, 2]);
}

#[test]
fn sort_top_k_clamps() {
    let out = sort_top_k(&[1.0, 2.0, 3.0], 10);
    assert_eq!(out, [2, 1, 0]);
}

#[test]
fn sort_top_k_empty() {
    assert!(sort_top_k(&[], 5).is_empty());
}

// --- rrf_scores ---

#[test]
fn rrf_assigns_by_rank() {
    let mut raw = Scores::new();
    raw.insert(0, 0.1);
    raw.insert(1, 0.9);
    raw.insert(2, 0.5);
    let rrf = rrf_scores(&raw);
    assert!((rrf[&1] - 1.0 / (RRF_K as f64 + 1.0)).abs() < 1e-12);
    assert!((rrf[&2] - 1.0 / (RRF_K as f64 + 2.0)).abs() < 1e-12);
    assert!((rrf[&0] - 1.0 / (RRF_K as f64 + 3.0)).abs() < 1e-12);
}

#[test]
fn rrf_empty() {
    assert!(rrf_scores(&Scores::new()).is_empty());
}

#[test]
fn rrf_first_rank_is_one_over_61() {
    let mut raw = Scores::new();
    raw.insert(0, 5.0);
    let rrf = rrf_scores(&raw);
    assert!((rrf[&0] - 1.0 / 61.0).abs() < 1e-12);
}

// --- search_semantic / search_bm25 ---

#[test]
fn semantic_distance_to_similarity() {
    let chunks = make_chunks();
    let idx = MockSemantic::new(vec![(0, 0.2), (2, 0.7)]);
    let results = search_semantic("q", &MockModel, &idx, &chunks, 5, None);
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, 0);
    assert!((results[0].1 - 0.8).abs() < 1e-10);
    assert_eq!(results[1].0, 2);
    assert!((results[1].1 - 0.3).abs() < 1e-10);
}

#[test]
fn semantic_passes_selector_and_k() {
    let chunks = make_chunks();
    let idx = MockSemantic::new(vec![(0, 0.5)]);
    let selector = vec![0u32, 2];
    search_semantic("q", &MockModel, &idx, &chunks, 5, Some(&selector));
    let calls = idx.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].selector.as_deref(), Some([0u32, 2].as_slice()));
    assert_eq!(calls[0].k, 5);
}

#[test]
fn bm25_excludes_zero_and_sorts() {
    let chunks = make_chunks();
    let bm = MockBm25::new(vec![0.5, 0.0, 0.9, 0.2, 0.0]);
    let results = search_bm25("alpha beta", &bm, &chunks, 5, None);
    let idxs: Vec<usize> = results.iter().map(|r| r.0).collect();
    assert_eq!(idxs, [2, 0, 3]);
    assert!((results[0].1 - 0.9).abs() < 1e-5);
}

#[test]
fn bm25_empty_tokens() {
    let chunks = make_chunks();
    let bm = MockBm25::new(vec![1.0; 5]);
    assert!(search_bm25("   ", &bm, &chunks, 5, None).is_empty());
}

#[test]
fn bm25_builds_mask_from_selector() {
    let chunks = make_chunks();
    let bm = MockBm25::new(vec![1.0; 5]);
    search_bm25("alpha", &bm, &chunks, 5, Some(&[1, 3]));
    let calls = bm.calls.borrow();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].mask.as_deref(), Some([0u8, 1, 0, 1, 0].as_slice()));
}

// --- search ---

#[test]
fn search_alpha_one_is_semantic() {
    let chunks = make_chunks();
    let idx = MockSemantic::new(vec![(2, 0.05), (0, 0.10)]);
    let bm = MockBm25::new(vec![0.0, 0.0, 0.0, 0.0, 9.0]);
    let results = search(
        "alpha",
        &MockModel,
        &idx,
        &bm,
        &chunks,
        3,
        &opts(Some(1.0), Some(false)),
    );
    assert_eq!(results[0].chunk, chunks[2]);
    assert_eq!(results[1].chunk, chunks[0]);
    assert!(results[0].score > 0.0);
    assert!(results[1].score > 0.0);
    // The BM25-only chunk has a fused score of 0.0 at alpha=1.0 and is dropped
    // (semble#219 filters `combined_scores` before ranking).
    assert!(results.iter().all(|r| r.chunk != chunks[4]));
}

#[test]
fn search_alpha_zero_is_bm25() {
    let chunks = make_chunks();
    let idx = MockSemantic::new(vec![(0, 0.05)]);
    let bm = MockBm25::new(vec![0.5, 0.0, 0.9, 0.2, 0.0]);
    let results = search(
        "alpha",
        &MockModel,
        &idx,
        &bm,
        &chunks,
        3,
        &opts(Some(0.0), Some(false)),
    );
    let got: Vec<&Chunk> = results.iter().map(|r| &r.chunk).collect();
    assert_eq!(got, vec![&chunks[2], &chunks[0], &chunks[3]]);
}

#[test]
fn search_rrf_first_rank_score() {
    let chunks = make_chunks();
    let idx = MockSemantic::new(vec![(0, 0.0)]);
    let bm = MockBm25::new(vec![0.0; 5]);
    let results = search(
        "q",
        &MockModel,
        &idx,
        &bm,
        &chunks,
        5,
        &opts(Some(1.0), Some(false)),
    );
    assert_eq!(results.len(), 1);
    assert!((results[0].score - 1.0 / 61.0).abs() < 1e-10);
}

#[test]
fn search_sorts_ties_by_start_line() {
    let chunks = vec![
        make_chunk("foo", "src/late.ts", 100, 100),
        make_chunk("bar", "src/early.ts", 1, 1),
    ];
    let idx = MockSemantic::new(vec![(0, 0.5)]);
    let bm = MockBm25::new(vec![0.0, 1.0]);
    let results = search(
        "q",
        &MockModel,
        &idx,
        &bm,
        &chunks,
        5,
        &opts(Some(0.5), Some(false)),
    );
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].chunk.start_line, 1);
    assert_eq!(results[1].chunk.start_line, 100);
}

#[test]
fn search_empty_inputs() {
    let chunks = make_chunks();
    let idx = MockSemantic::new(vec![]);
    let bm = MockBm25::new(vec![0.0; 5]);
    let results = search(
        "q",
        &MockModel,
        &idx,
        &bm,
        &chunks,
        5,
        &SearchOptions::default(),
    );
    assert!(results.is_empty());
}

#[test]
fn search_rerank_applies_multi_chunk_boost() {
    let chunks = make_chunks();
    let idx = MockSemantic::new(vec![(2, 0.05), (0, 0.10), (1, 0.20)]);
    let bm = MockBm25::new(vec![0.0; 5]);
    let unranked = search(
        "q",
        &MockModel,
        &idx,
        &bm,
        &chunks,
        3,
        &opts(Some(1.0), Some(false)),
    );
    assert_eq!(unranked[0].chunk.file_path, "src/gamma.ts");

    let ranked = search(
        "q",
        &MockModel,
        &idx,
        &bm,
        &chunks,
        3,
        &opts(Some(1.0), Some(true)),
    );
    assert_eq!(ranked[0].chunk.file_path, "src/alpha.ts");
}
