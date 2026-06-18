//! Minimal BM25 index + BM25 enrichment. Port of `src/indexing/sparse.ts`
//! (← semble `index/sparse.py`, standing in for Python's `bm25s`).
//!
//! Phase 1 covers the pure scoring core: `enrich_for_bm25`, `selector_to_mask`,
//! and `Bm25Index::{build, get_scores}`. On-disk `save`/`load` (filesystem) is
//! deferred to Phase 3 (T014); the state derives serde so it can be added
//! without reshaping.
//!
//! Float parity: the upstream stores scores in a `Float32Array`, so each
//! additive accumulation is rounded to `f32`. We reproduce that exactly —
//! `score = ((score as f64) + contrib) as f32` — and iterate unique query terms
//! in first-appearance order (JS `Set` insertion order), since `f32`
//! accumulation is order-sensitive.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::types::Chunk;

// Standard Okapi BM25 hyperparameters (bm25s' default Lucene scorer).
const K1: f64 = 1.5;
const B: f64 = 0.75;

/// Node `path.posix.parse(base).name`: the basename without its final
/// extension, leaving a leading-dot filename (`.gitignore`) untouched.
fn stem_of(base: &str) -> &str {
    match base.rfind('.') {
        Some(0) | None => base,
        Some(i) => &base[..i],
    }
}

/// Append file-path components to BM25 content to boost path-based queries.
///
/// The stem is repeated twice to up-weight path matches; the last three
/// directory parts follow. Backslashes are normalized to POSIX first so a
/// Windows-host index produces the same enriched text as a POSIX host.
pub fn enrich_for_bm25(chunk: &Chunk) -> String {
    let normalized = chunk.file_path.replace('\\', "/");
    let (dir, base) = match normalized.rfind('/') {
        Some(i) => (&normalized[..i], &normalized[i + 1..]),
        None => ("", normalized.as_str()),
    };
    let stem = stem_of(base);
    let parts: Vec<&str> = dir
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    let start = parts.len().saturating_sub(3);
    let dir_text = parts[start..].join(" ");
    format!("{} {stem} {stem} {dir_text}", chunk.content)
}

/// Convert a selector of indices into a 0/1 mask of length `size`, or `None`
/// when the selector is absent. Out-of-bounds indices are silently dropped.
pub fn selector_to_mask(selector: Option<&[u32]>, size: usize) -> Option<Vec<u8>> {
    selector.map(|sel| {
        let mut mask = vec![0u8; size];
        for &idx in sel {
            if (idx as usize) < size {
                mask[idx as usize] = 1;
            }
        }
        mask
    })
}

/// Minimal in-memory BM25 index supporting `build` and `get_scores`.
///
/// Documents are passed pre-tokenized (callers use
/// `tokenize(&enrich_for_bm25(chunk))`). `get_scores` returns per-document
/// scores in document order, matching `bm25s.BM25.get_scores`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bm25Index {
    num_docs: usize,
    /// Token count per document, in document order.
    doc_lengths: Vec<f32>,
    avg_doc_length: f64,
    /// term -> postings list of `(doc_id, term_freq)`.
    postings: HashMap<String, Vec<(usize, u32)>>,
    /// term -> document frequency.
    doc_freq: HashMap<String, u32>,
}

impl Bm25Index {
    /// Build an index from pre-tokenized documents.
    pub fn build(documents: &[Vec<String>]) -> Self {
        let num_docs = documents.len();
        let mut doc_lengths = vec![0f32; num_docs];
        let mut postings: HashMap<String, Vec<(usize, u32)>> = HashMap::new();
        let mut doc_freq: HashMap<String, u32> = HashMap::new();

        let mut total_len = 0usize;
        for (doc_id, tokens) in documents.iter().enumerate() {
            doc_lengths[doc_id] = tokens.len() as f32;
            total_len += tokens.len();

            // Term frequencies for this document, in first-appearance order so
            // the postings list order matches the upstream `Map` iteration.
            let mut tf_order: Vec<String> = Vec::new();
            let mut tf: HashMap<&str, u32> = HashMap::new();
            for token in tokens {
                let entry = tf.entry(token.as_str()).or_insert(0);
                if *entry == 0 {
                    tf_order.push(token.clone());
                }
                *entry += 1;
            }

            for term in tf_order {
                let freq = tf[term.as_str()];
                postings
                    .entry(term.clone())
                    .or_default()
                    .push((doc_id, freq));
                *doc_freq.entry(term).or_insert(0) += 1;
            }
        }

        let avg_doc_length = if num_docs > 0 {
            total_len as f64 / num_docs as f64
        } else {
            0.0
        };

        Self {
            num_docs,
            doc_lengths,
            avg_doc_length,
            postings,
            doc_freq,
        }
    }

    /// Number of indexed documents.
    pub fn num_docs(&self) -> usize {
        self.num_docs
    }

    /// Compute BM25 scores for the query tokens, in document order.
    ///
    /// When `weight_mask` is provided, documents with `mask[i] == 0` score 0
    /// (matching `bm25s.BM25.get_scores(..., weight_mask=mask)`).
    pub fn get_scores(&self, query_tokens: &[String], weight_mask: Option<&[u8]>) -> Vec<f32> {
        let mut scores = vec![0f32; self.num_docs];
        if query_tokens.is_empty() || self.num_docs == 0 {
            return scores;
        }

        // De-duplicate query terms, preserving first-appearance order so the
        // order-sensitive f32 accumulation matches the upstream `Set`.
        let mut seen: HashSet<&str> = HashSet::new();
        let mut unique: Vec<&str> = Vec::new();
        for token in query_tokens {
            if seen.insert(token.as_str()) {
                unique.push(token.as_str());
            }
        }

        for term in unique {
            let Some(list) = self.postings.get(term) else {
                continue;
            };
            let df = self.doc_freq.get(term).copied().unwrap_or(0);
            // Lucene/Robertson IDF: log(1 + (N - df + 0.5) / (df + 0.5)).
            let idf = (1.0 + (self.num_docs as f64 - df as f64 + 0.5) / (df as f64 + 0.5)).ln();

            for &(doc_id, freq) in list {
                if let Some(mask) = weight_mask {
                    if mask.get(doc_id).copied().unwrap_or(0) == 0 {
                        continue;
                    }
                }
                let dl = doc_lengths_get(&self.doc_lengths, doc_id);
                let avg = if self.avg_doc_length != 0.0 {
                    self.avg_doc_length
                } else {
                    1.0
                };
                let denom = freq as f64 + K1 * (1.0 - B + (B * dl) / avg);
                let denom = if denom != 0.0 { denom } else { 1.0 };
                let contrib = (idf * (freq as f64 * (K1 + 1.0))) / denom;
                // Float32 accumulation (mirrors the Float32Array store).
                scores[doc_id] = ((scores[doc_id] as f64) + contrib) as f32;
            }
        }

        scores
    }
}

fn doc_lengths_get(doc_lengths: &[f32], doc_id: usize) -> f64 {
    doc_lengths.get(doc_id).copied().unwrap_or(0.0) as f64
}

#[cfg(test)]
mod tests {
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
}
