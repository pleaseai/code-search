//! Indexing. Port of `src/indexing/*` (← semble `index/`).
//!
//! Phase 1 lands the pure BM25 scoring core (`sparse`). File walking, dense
//! embeddings, the content-hash cache, and on-disk persistence arrive in
//! Phase 3.

pub mod sparse;
