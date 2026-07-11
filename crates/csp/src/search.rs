//! Hybrid search pipeline. Port of semble `search.py`.
//!
//! semantic + BM25 → per-list RRF (`k=60`) → alpha-weighted combine → optional
//! rerank. The rerank stage mirrors the upstream `search.search` order:
//! multi-chunk file boost (`boost_multi_chunk_files`), then query-type boost
//! (`apply_query_boost`), then top-k with path penalties + file saturation
//! (`rerank_top_k`, with `penalise_paths = alpha_weight < 1.0`).

use std::collections::HashSet;

use crate::indexing::sparse::selector_to_mask;
use crate::ranking::boosting::{apply_query_boost, boost_multi_chunk_files};
use crate::ranking::penalties::rerank_top_k;
use crate::ranking::weighting::resolve_alpha;
use crate::ranking::Scores;
use crate::tokens::tokenize;
use crate::types::Chunk;

/// Reciprocal Rank Fusion constant.
pub const RRF_K: usize = 60;

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
    // Drop chunks whose fused score is exactly 0.0 before ranking (parity with
    // semble#219's `combined_scores = {... if score}`).
    combined.retain(|_, &mut score| score != 0.0);

    let ranked: Vec<(usize, f64)> = if rerank {
        boost_multi_chunk_files(&mut combined, chunks);
        let boosted = apply_query_boost(&combined, query, chunks);
        // Path penalties apply only when BM25 contributes (alpha_weight < 1.0).
        rerank_top_k(&boosted, chunks, top_k, alpha_weight < 1.0)
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
mod tests;
