//! Hybrid search pipeline. Port of `src/search.ts` (← semble `search.py`).
//!
//! semantic + BM25 → per-list RRF (`k=60`) → alpha-weighted combine → optional
//! rerank (multi-chunk file boost → query boost → top-k with file saturation).
//!
//! Parity note: like `search.ts`, this reproduces the module's *current* inline
//! ranking — `apply_query_boost` is an identity pass and `rerank_top_k` applies
//! only file-saturation decay (no path penalties). The fuller
//! `ranking::{boosting::apply_query_boost, penalties::rerank_top_k}` are ported
//! (T006/T007) but, exactly as in the TS source, are not yet wired into the
//! search pipeline (`TODO(integration)`). `boost_multi_chunk_files` *is* the
//! shared ranking implementation (identical to the TS inline version).

use std::collections::HashSet;

use indexmap::IndexMap;

use crate::indexing::sparse::selector_to_mask;
use crate::ranking::boosting::boost_multi_chunk_files;
use crate::ranking::weighting::resolve_alpha;
use crate::ranking::Scores;
use crate::tokens::tokenize;
use crate::types::Chunk;

/// Reciprocal Rank Fusion constant.
pub const RRF_K: usize = 60;

const FILE_SATURATION_THRESHOLD: usize = 1;
const FILE_SATURATION_DECAY: f64 = 0.5;

/// A scored search hit.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub chunk: Chunk,
    pub score: f64,
}

/// Embedding model (parallels `model2vec.StaticModel`).
pub trait EmbeddingModel {
    fn encode(&self, texts: &[String]) -> Vec<Vec<f32>>;
}

/// Vector backend (parallels `vicinity` cosine backend). `query` returns one
/// result list per query vector — `[(chunk_index, cosine_distance)]` ascending.
pub trait VectorBackend {
    fn query(
        &self,
        vectors: &[Vec<f32>],
        k: usize,
        selector: Option<&[u32]>,
    ) -> Vec<Vec<(usize, f64)>>;
}

/// Sparse backend (parallels `bm25s.BM25`).
pub trait SparseBackend {
    fn get_scores(&self, query_tokens: &[String], weight_mask: Option<&[u8]>) -> Vec<f32>;
}

impl EmbeddingModel for crate::indexing::dense::Model {
    fn encode(&self, texts: &[String]) -> Vec<Vec<f32>> {
        crate::indexing::dense::Model::encode(self, texts)
    }
}

impl VectorBackend for crate::indexing::dense::SelectableBasicBackend {
    fn query(
        &self,
        vectors: &[Vec<f32>],
        k: usize,
        selector: Option<&[u32]>,
    ) -> Vec<Vec<(usize, f64)>> {
        // A backend query error (dimension mismatch, bad selector) is an internal
        // invariant break, but in the hot search path / long-running MCP server we
        // degrade to no semantic hits rather than panicking the whole process.
        match crate::indexing::dense::SelectableBasicBackend::query(self, vectors, k, selector) {
            Ok(results) => results,
            Err(e) => {
                eprintln!("csp: vector backend query failed: {e}");
                Vec::new()
            }
        }
    }
}

impl SparseBackend for crate::indexing::sparse::Bm25Index {
    fn get_scores(&self, query_tokens: &[String], weight_mask: Option<&[u8]>) -> Vec<f32> {
        crate::indexing::sparse::Bm25Index::get_scores(self, query_tokens, weight_mask)
    }
}

/// Convert raw scores to RRF scores `1 / (RRF_K + rank)`; highest raw score →
/// rank 1. Ties break by insertion order (stable sort).
pub fn rrf_scores(scores: &Scores) -> Scores {
    if scores.is_empty() {
        return scores.clone();
    }
    let mut ranked: Vec<(usize, f64)> = scores.iter().map(|(&i, &s)| (i, s)).collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    let mut out = Scores::new();
    for (rank0, (idx, _)) in ranked.into_iter().enumerate() {
        out.insert(idx, 1.0 / (RRF_K as f64 + (rank0 + 1) as f64));
    }
    out
}

/// Indices of the top-k largest entries of `arr`, descending; ties by index.
pub fn sort_top_k(arr: &[f32], top_k: usize) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..arr.len()).collect();
    indices.sort_by(|&a, &b| arr[b].total_cmp(&arr[a]));
    indices.truncate(top_k.min(arr.len()));
    indices
}

/// Semantic search: cosine distance → similarity (`1 - distance`).
pub fn search_semantic(
    query: &str,
    model: &impl EmbeddingModel,
    semantic_index: &impl VectorBackend,
    chunks: &[Chunk],
    top_k: usize,
    selector: Option<&[u32]>,
) -> Vec<(usize, f64)> {
    let query_embedding = model.encode(&[query.to_string()]);
    let batch = semantic_index.query(&query_embedding, top_k, selector);
    let Some(first) = batch.into_iter().next() else {
        return Vec::new();
    };
    first
        .into_iter()
        .filter(|&(index, _)| index < chunks.len())
        .map(|(index, distance)| (index, 1.0 - distance))
        .collect()
}

/// BM25 search: chunks ranked by score, excluding zero/negative scores.
pub fn search_bm25(
    query: &str,
    bm25_index: &impl SparseBackend,
    chunks: &[Chunk],
    top_k: usize,
    selector: Option<&[u32]>,
) -> Vec<(usize, f64)> {
    let tokens = tokenize(query);
    if tokens.is_empty() {
        return Vec::new();
    }
    let mask = selector_to_mask(selector, chunks.len());
    let scores = bm25_index.get_scores(&tokens, mask.as_deref());
    let mut results = Vec::new();
    for i in sort_top_k(&scores, top_k) {
        let score = scores[i];
        if score <= 0.0 || i >= chunks.len() {
            continue;
        }
        results.push((i, score as f64));
    }
    results
}

/// Search options.
#[derive(Debug, Clone, Default)]
pub struct SearchOptions {
    /// Semantic weight (`1 - alpha` for BM25); `None` auto-detects by query type.
    pub alpha: Option<f64>,
    /// Chunk-index selector to filter candidates.
    pub selector: Option<Vec<u32>>,
    /// Apply code-tuned reranking. `None` defaults to `true`.
    pub rerank: Option<bool>,
}

/// Identity query boost — mirrors the current `search.ts` inline stub. (The full
/// `ranking::boosting::apply_query_boost` is ported but not yet wired here.)
fn apply_query_boost_identity(scores: &Scores) -> Scores {
    scores.clone()
}

/// Top-k rerank with file-saturation decay only — mirrors the current `search.ts`
/// inline stub (path penalties not applied; the `penalise_paths` flag is ignored,
/// matching the TS `void options`).
fn rerank_top_k_saturation(scores: &Scores, chunks: &[Chunk], top_k: usize) -> Vec<(usize, f64)> {
    if scores.is_empty() {
        return Vec::new();
    }
    let mut ranked: Vec<(usize, f64)> = scores.iter().map(|(&i, &s)| (i, s)).collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));

    let mut file_selected: IndexMap<String, usize> = IndexMap::new();
    let mut selected: Vec<(f64, usize)> = Vec::new();
    let mut min_selected = f64::INFINITY;

    for (idx, pen_score) in ranked {
        if selected.len() >= top_k && pen_score <= min_selected {
            break;
        }
        let already = file_selected
            .get(&chunks[idx].file_path)
            .copied()
            .unwrap_or(0);
        let mut eff_score = pen_score;
        if already >= FILE_SATURATION_THRESHOLD {
            let excess = already - FILE_SATURATION_THRESHOLD + 1;
            eff_score *= FILE_SATURATION_DECAY.powi(excess as i32);
        }
        selected.push((eff_score, idx));
        file_selected.insert(chunks[idx].file_path.clone(), already + 1);
        if selected.len() >= top_k {
            min_selected = selected
                .iter()
                .map(|&(s, _)| s)
                .fold(f64::INFINITY, f64::min);
        }
    }

    selected.sort_by(|a, b| b.0.total_cmp(&a.0));
    selected.truncate(top_k);
    selected
        .into_iter()
        .map(|(score, idx)| (idx, score))
        .collect()
}

/// Hybrid search: alpha-weighted combination of RRF-normalised semantic and BM25
/// scores, with optional code-tuned reranking.
pub fn search(
    query: &str,
    model: &impl EmbeddingModel,
    semantic_index: &impl VectorBackend,
    bm25_index: &impl SparseBackend,
    chunks: &[Chunk],
    top_k: usize,
    options: &SearchOptions,
) -> Vec<SearchResult> {
    let alpha_weight = resolve_alpha(query, options.alpha);
    let rerank = options.rerank.unwrap_or(true);
    let selector = options.selector.as_deref();

    // Over-fetch so the merged pool is large enough after union & re-ranking.
    let candidate_count = top_k * 5;

    let mut semantic_scores = Scores::new();
    for (idx, score) in search_semantic(
        query,
        model,
        semantic_index,
        chunks,
        candidate_count,
        selector,
    ) {
        semantic_scores.insert(idx, score);
    }

    let mut bm25_scores = Scores::new();
    for (idx, score) in search_bm25(query, bm25_index, chunks, candidate_count, selector) {
        if score != 0.0 {
            bm25_scores.insert(idx, score);
        }
    }

    let normalized_semantic = rrf_scores(&semantic_scores);
    let normalized_bm25 = rrf_scores(&bm25_scores);

    // Union, then sort by start_line to counteract hash-iteration nondeterminism.
    let mut seen: HashSet<usize> = HashSet::new();
    let mut union: Vec<usize> = Vec::new();
    for &idx in normalized_semantic.keys().chain(normalized_bm25.keys()) {
        if seen.insert(idx) {
            union.push(idx);
        }
    }
    union.sort_by(|&a, &b| chunks[a].start_line.cmp(&chunks[b].start_line));

    let mut combined = Scores::new();
    for &idx in &union {
        let s = normalized_semantic.get(&idx).copied().unwrap_or(0.0);
        let b = normalized_bm25.get(&idx).copied().unwrap_or(0.0);
        combined.insert(idx, alpha_weight * s + (1.0 - alpha_weight) * b);
    }

    let ranked: Vec<(usize, f64)> = if rerank {
        boost_multi_chunk_files(&mut combined, chunks);
        let boosted = apply_query_boost_identity(&combined);
        rerank_top_k_saturation(&boosted, chunks, top_k)
    } else {
        let mut entries: Vec<(usize, f64)> = combined.iter().map(|(&i, &s)| (i, s)).collect();
        entries.sort_by(|a, b| b.1.total_cmp(&a.1));
        entries.truncate(top_k);
        entries
    };

    ranked
        .into_iter()
        .map(|(idx, score)| SearchResult {
            chunk: chunks[idx].clone(),
            score,
        })
        .collect()
}

#[cfg(test)]
mod tests {
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
        if let Some(r) = results.iter().find(|r| r.chunk == chunks[4]) {
            assert_eq!(r.score, 0.0);
        }
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
        let idx = MockSemantic::new(vec![(0, 0.10), (1, 0.20), (2, 0.30)]);
        let bm = MockBm25::new(vec![0.0; 5]);
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
}
