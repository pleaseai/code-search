//! BM25 index + BM25 enrichment. Port of semble `index/bm25.py` (the own
//! incremental `BM25` class that replaced `bm25s` in upstream #225) plus
//! `index/sparse.py` (`enrich_for_bm25`).
//!
//! `Bm25Index` keys documents on a stable chunk id (`"{path}:{slot}"`, see
//! `indexing::types::make_chunk_id`) and supports `add_document` /
//! `remove_document` so an incremental reindex can replace one file's postings
//! without rebuilding the corpus. `set_doc_order` fixes the global chunk-list
//! order that `get_scores` output is aligned to. Persistence (`bm25.json`)
//! stores the per-document term counts plus that order; postings are rebuilt
//! on load.
//!
//! Float parity: scores accumulate in `f32` (upstream uses a `float32` numpy
//! array), so each addition is rounded to `f32` and unique query terms are
//! visited in first-appearance order, since `f32` accumulation is
//! order-sensitive.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

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

/// One indexed document: its id, term counts, and token length.
#[derive(Debug, Clone)]
struct Doc {
    chunk_id: String,
    /// term → term frequency, in first-appearance order.
    terms: Vec<(String, u32)>,
    length: usize,
}

/// Incremental in-memory BM25 index keyed on stable chunk ids.
///
/// Documents are passed pre-tokenized (callers use
/// `tokenize(&enrich_for_bm25(chunk))`). `get_scores` returns one score per
/// entry of the current document order (see [`set_doc_order`](Self::set_doc_order)).
#[derive(Debug, Clone, Default)]
pub struct Bm25Index {
    /// chunk id → internal slot.
    ids: HashMap<String, u32>,
    /// slot → document (`None` after removal; slots are recycled).
    docs: Vec<Option<Doc>>,
    free_slots: Vec<u32>,
    /// term → (slot → term frequency).
    postings: HashMap<String, HashMap<u32, u32>>,
    total_doc_length: usize,
    /// Global chunk-list order that `get_scores` output is aligned to.
    doc_order: Vec<String>,
    /// slot → position in `doc_order` (`None` when not in the current order).
    order_positions: Vec<Option<usize>>,
}

impl Bm25Index {
    /// Create an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build an index from pre-tokenized documents with positional ids
    /// (`"0"`, `"1"`, …) and a matching document order. Convenience for callers
    /// that do not track per-file chunk ids (tests, fixtures).
    pub fn build(documents: &[Vec<String>]) -> Self {
        let mut index = Self::new();
        let mut order = Vec::with_capacity(documents.len());
        for (i, tokens) in documents.iter().enumerate() {
            let chunk_id = i.to_string();
            index
                .add_document(&chunk_id, tokens)
                .expect("positional ids are unique");
            order.push(chunk_id);
        }
        index.set_doc_order(order);
        index
    }

    /// Index one document, rejecting a duplicate chunk id.
    pub fn add_document(&mut self, chunk_id: &str, tokens: &[String]) -> Result<(), String> {
        if self.ids.contains_key(chunk_id) {
            return Err(format!("chunk_id already indexed: {chunk_id}"));
        }
        // Term frequencies in first-appearance order.
        let mut terms: Vec<(String, u32)> = Vec::new();
        let mut positions: HashMap<&str, usize> = HashMap::new();
        for token in tokens {
            match positions.get(token.as_str()) {
                Some(&i) => terms[i].1 += 1,
                None => {
                    positions.insert(token.as_str(), terms.len());
                    terms.push((token.clone(), 1));
                }
            }
        }

        let slot = match self.free_slots.pop() {
            Some(slot) => slot,
            None => {
                self.docs.push(None);
                self.order_positions.push(None);
                (self.docs.len() - 1) as u32
            }
        };
        for (term, freq) in &terms {
            self.postings
                .entry(term.clone())
                .or_default()
                .insert(slot, *freq);
        }
        self.total_doc_length += tokens.len();
        self.ids.insert(chunk_id.to_string(), slot);
        self.order_positions[slot as usize] = None;
        self.docs[slot as usize] = Some(Doc {
            chunk_id: chunk_id.to_string(),
            terms,
            length: tokens.len(),
        });
        Ok(())
    }

    /// Remove a document's postings; no-op when `chunk_id` is not indexed.
    pub fn remove_document(&mut self, chunk_id: &str) {
        let Some(slot) = self.ids.remove(chunk_id) else {
            return;
        };
        let Some(doc) = self.docs[slot as usize].take() else {
            return;
        };
        self.total_doc_length -= doc.length;
        for (term, _) in &doc.terms {
            if let Some(docs) = self.postings.get_mut(term) {
                docs.remove(&slot);
                if docs.is_empty() {
                    self.postings.remove(term);
                }
            }
        }
        self.order_positions[slot as usize] = None;
        self.free_slots.push(slot);
    }

    /// Set the global chunk-list order that `get_scores` output is aligned to.
    /// Ids not (or no longer) indexed simply score 0 at their position.
    pub fn set_doc_order(&mut self, chunk_ids: Vec<String>) {
        for position in self.order_positions.iter_mut() {
            *position = None;
        }
        for (i, chunk_id) in chunk_ids.iter().enumerate() {
            if let Some(&slot) = self.ids.get(chunk_id) {
                self.order_positions[slot as usize] = Some(i);
            }
        }
        self.doc_order = chunk_ids;
    }

    /// The current document order (aligned with `get_scores` output).
    pub fn doc_order(&self) -> &[String] {
        &self.doc_order
    }

    /// Number of entries in the document order — the length of `get_scores`.
    pub fn num_docs(&self) -> usize {
        self.doc_order.len()
    }

    /// Number of indexed documents (the BM25 corpus size).
    pub fn corpus_size(&self) -> usize {
        self.ids.len()
    }

    /// Compute BM25 scores for the query tokens, aligned with the document order.
    ///
    /// When `weight_mask` is provided, positions with `mask[i] == 0` score 0
    /// (matching upstream `BM25.get_scores(..., weight_mask=mask)`).
    pub fn get_scores(&self, query_tokens: &[String], weight_mask: Option<&[u8]>) -> Vec<f32> {
        let mut scores = vec![0f32; self.doc_order.len()];
        let corpus_size = self.corpus_size();
        if query_tokens.is_empty() || corpus_size == 0 {
            return scores;
        }

        // De-duplicate query terms, preserving first-appearance order so the
        // order-sensitive f32 accumulation is deterministic.
        let mut seen: HashSet<&str> = HashSet::new();
        let mut unique: Vec<&str> = Vec::new();
        for token in query_tokens {
            if seen.insert(token.as_str()) {
                unique.push(token.as_str());
            }
        }

        let avg = self.total_doc_length as f64 / corpus_size as f64;
        let avg = if avg != 0.0 { avg } else { 1.0 };
        for term in unique {
            let Some(docs) = self.postings.get(term) else {
                continue;
            };
            let df = docs.len() as f64;
            // Lucene/Robertson IDF: log(1 + (N - df + 0.5) / (df + 0.5)).
            let idf = (1.0 + (corpus_size as f64 - df + 0.5) / (df + 0.5)).ln();

            for (&slot, &freq) in docs {
                let Some(position) = self.order_positions[slot as usize] else {
                    continue;
                };
                if let Some(mask) = weight_mask {
                    if mask.get(position).copied().unwrap_or(0) == 0 {
                        continue;
                    }
                }
                let dl = self.docs[slot as usize]
                    .as_ref()
                    .map_or(0.0, |doc| doc.length as f64);
                let denom = freq as f64 + K1 * (1.0 - B + (B * dl) / avg);
                let denom = if denom != 0.0 { denom } else { 1.0 };
                let contrib = (idf * (freq as f64 * (K1 + 1.0))) / denom;
                // Float32 accumulation (mirrors the float32 score array).
                scores[position] = ((scores[position] as f64) + contrib) as f32;
            }
        }

        scores
    }

    /// Persist the index to `dir/bm25.json`, creating `dir` if needed.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let documents: BTreeMap<&str, BTreeMap<&str, u32>> = self
            .docs
            .iter()
            .flatten()
            .map(|doc| {
                let counts = doc
                    .terms
                    .iter()
                    .map(|(term, freq)| (term.as_str(), *freq))
                    .collect();
                (doc.chunk_id.as_str(), counts)
            })
            .collect();
        let serialized = Bm25Serialized {
            version: BM25_FORMAT_VERSION,
            documents,
            doc_order: self.doc_order.iter().map(String::as_str).collect(),
        };
        let json = serde_json::to_string(&serialized)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(dir.join("bm25.json"), json)
    }

    /// Load an index previously persisted with [`save`](Self::save),
    /// reconstructing its postings. Errors when the persisted document order
    /// does not describe exactly the persisted documents.
    pub fn load(dir: &Path) -> std::io::Result<Self> {
        let raw = std::fs::read_to_string(dir.join("bm25.json"))?;
        let parsed: Bm25Serialized<String> = serde_json::from_str(&raw)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let invalid = |msg: &str| std::io::Error::new(std::io::ErrorKind::InvalidData, msg);
        if parsed.version != BM25_FORMAT_VERSION {
            return Err(invalid(&format!(
                "Unsupported BM25 format {}; expected {BM25_FORMAT_VERSION}",
                parsed.version
            )));
        }
        let order_set: HashSet<&str> = parsed.doc_order.iter().map(String::as_str).collect();
        let document_set: HashSet<&str> = parsed.documents.keys().map(String::as_str).collect();
        if order_set.len() != parsed.doc_order.len() || order_set != document_set {
            return Err(invalid("Persisted BM25 document state is inconsistent"));
        }

        let mut index = Self::new();
        for (chunk_id, counts) in parsed.documents {
            let mut tokens: Vec<String> = Vec::new();
            for (term, freq) in counts {
                tokens.extend(std::iter::repeat_n(term, freq as usize));
            }
            index
                .add_document(&chunk_id, &tokens)
                .map_err(|e| invalid(&e))?;
        }
        index.set_doc_order(parsed.doc_order);
        Ok(index)
    }
}

/// On-disk format version of `bm25.json`. v1 was the positional
/// (`numDocs`/`docLengths`/`postings`) layout that predates stable chunk ids.
const BM25_FORMAT_VERSION: u32 = 2;

/// On-disk representation of [`Bm25Index`]: per-document term counts plus the
/// document order (postings are derived on load). Maps are `BTreeMap` so the
/// serialized bytes are deterministic.
#[derive(Serialize, Deserialize)]
struct Bm25Serialized<S: Ord> {
    version: u32,
    documents: BTreeMap<S, BTreeMap<S, u32>>,
    #[serde(rename = "docOrder")]
    doc_order: Vec<S>,
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
}
