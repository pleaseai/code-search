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

/// One indexed document: its term counts and token length. The document's
/// chunk id lives once, as the `Bm25Index::ids` key.
#[derive(Debug, Clone)]
struct Doc {
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
        self.insert_document(chunk_id, terms, tokens.len())
    }

    /// Index one document from its term counts directly — the shape `bm25.json`
    /// persists, so `load` never has to materialise a token stream.
    fn insert_document(
        &mut self,
        chunk_id: &str,
        terms: Vec<(String, u32)>,
        length: usize,
    ) -> Result<(), String> {
        if self.ids.contains_key(chunk_id) {
            return Err(format!("chunk_id already indexed: {chunk_id}"));
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
        self.total_doc_length += length;
        self.ids.insert(chunk_id.to_string(), slot);
        self.order_positions[slot as usize] = None;
        self.docs[slot as usize] = Some(Doc { terms, length });
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
            .ids
            .iter()
            .filter_map(|(chunk_id, &slot)| {
                let doc = self.docs[slot as usize].as_ref()?;
                let counts = doc
                    .terms
                    .iter()
                    .map(|(term, freq)| (term.as_str(), *freq))
                    .collect();
                Some((chunk_id.as_str(), counts))
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
            // Sum in u64: `length` is derived from untrusted on-disk counts, and
            // an overflowing document must be rejected, not silently wrapped.
            let mut length = 0u64;
            let mut terms: Vec<(String, u32)> = Vec::with_capacity(counts.len());
            for (term, freq) in counts {
                // A zero count would still create a posting and inflate the
                // term's document frequency; treat it as corruption.
                if freq == 0 {
                    return Err(invalid("Persisted BM25 term frequencies must be positive"));
                }
                length += u64::from(freq);
                terms.push((term, freq));
            }
            let length = usize::try_from(length)
                .map_err(|_| invalid("Persisted BM25 document length is out of range"))?;
            index
                .insert_document(&chunk_id, terms, length)
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
mod tests;
