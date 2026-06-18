//! `csp` — hybrid code-search core library.
//!
//! Rust rewrite of `@pleaseai/csp` (see ADR-0003). This crate is the **library
//! seam**: the Rust-native successor of the former TypeScript `CspIndex`, and
//! the future napi-rs binding surface should the JS library contract return.
//!
//! Phase 1 (pure core) modules land first; later phases add chunking, indexing,
//! and search per the ADR-0003 roadmap.

pub mod chunking;
pub mod indexing;
pub mod ranking;
pub mod tokens;
pub mod types;
pub mod utils;
// pub mod indexing;   // Phase 3
// pub mod search;     // Phase 4
