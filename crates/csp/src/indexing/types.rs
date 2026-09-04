//! Incremental-reindex types. Port of semble `index/types.py` (upstream #225).
//!
//! Upstream keys the per-file manifest on `mtime_ns`; csp keys it on a
//! per-file content hash instead, matching the whole-tree content-hash oracle
//! `cache_orchestrator` already uses (see ADR-0005).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::indexing::sparse::Bm25Index;
use crate::types::Chunk;

/// Stable BM25 document id for a file chunk: `"{indexed_path}:{slot}"`.
pub fn make_chunk_id(indexed_path: &str, slot: usize) -> String {
    format!("{indexed_path}:{slot}")
}

/// A file's content hash and its chunk range within the global chunk list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileManifestEntry {
    /// sha256 (hex) of the file bytes at index time.
    pub hash: String,
    /// Index of the file's first chunk in the global chunk list.
    pub start: usize,
    /// Number of chunks produced for the file (may be 0).
    pub count: usize,
}

impl FileManifestEntry {
    /// Exclusive end of the chunk range.
    pub fn end(&self) -> usize {
        self.start + self.count
    }
}

/// Per-file manifest keyed by the indexed (display-root-relative) path.
pub type FileManifest = BTreeMap<String, FileManifestEntry>;

/// A previously built index, loaded for reuse during incremental reindexing.
#[derive(Debug)]
pub struct PreviousIndex {
    pub chunks: Vec<Chunk>,
    /// L2-normalised rows, aligned with `chunks`.
    pub vectors: Vec<Vec<f32>>,
    pub files: FileManifest,
    pub bm25_index: Bm25Index,
}

impl PreviousIndex {
    /// Assemble a `PreviousIndex`, verifying that chunks, vectors, the file
    /// manifest, and the BM25 document order all describe the same layout
    /// (mirrors the alignment checks in upstream `load_previous_for_incremental`).
    pub fn try_new(
        chunks: Vec<Chunk>,
        vectors: Vec<Vec<f32>>,
        files: FileManifest,
        bm25_index: Bm25Index,
    ) -> Result<Self, String> {
        let chunk_count = chunks.len();
        if chunk_count != vectors.len() || chunk_count != bm25_index.doc_order().len() {
            return Err("Persisted index components have inconsistent document counts".into());
        }
        if files.is_empty() {
            return Err("Persisted index has no file manifest".into());
        }

        // Entries must tile [0, chunk_count) exactly, in ascending `start` order.
        let mut entries: Vec<(&String, &FileManifestEntry)> = files.iter().collect();
        entries.sort_by_key(|(_, entry)| entry.start);
        let mut expected_ids: Vec<String> = Vec::with_capacity(chunk_count);
        let mut next_start = 0usize;
        for (indexed_path, entry) in entries {
            if entry.start != next_start || entry.end() > chunk_count {
                return Err(format!(
                    "File manifest entry for {indexed_path} does not tile the chunk list"
                ));
            }
            if chunks[entry.start..entry.end()]
                .iter()
                .any(|chunk| chunk.file_path != *indexed_path)
            {
                return Err(format!(
                    "Chunks in the range recorded for {indexed_path} belong to another file"
                ));
            }
            expected_ids.extend((0..entry.count).map(|slot| make_chunk_id(indexed_path, slot)));
            next_start = entry.end();
        }
        if next_start != chunk_count {
            return Err("File manifest does not cover every chunk".into());
        }
        if bm25_index.doc_order() != expected_ids.as_slice() {
            return Err("BM25 document order does not match the file manifest".into());
        }

        Ok(Self {
            chunks,
            vectors,
            files,
            bm25_index,
        })
    }
}
