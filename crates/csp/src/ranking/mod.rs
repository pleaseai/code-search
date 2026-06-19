//! Ranking pipeline. Port of `src/ranking/*` (← semble `ranking/`).
//!
//! Score maps are keyed by chunk **index** into a canonical `&[Chunk]` slice and
//! use [`indexmap::IndexMap`] to preserve insertion order — the Rust counterpart
//! of the TypeScript `Map<Chunk, number>` keyed by object identity (whose
//! iteration order, and thus tie-breaking, the upstream code relies on).

use indexmap::IndexMap;

pub mod boosting;
pub mod penalties;
pub mod weighting;

/// Candidate scores keyed by chunk index, insertion-ordered.
pub type Scores = IndexMap<usize, f64>;
