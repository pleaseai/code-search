//! `csp` — hybrid code-search core library.
//!
//! Rust rewrite of `@pleaseai/csp` (see ADR-0003). This crate is the **library
//! seam**: the Rust-native successor of the former TypeScript `CspIndex`, and
//! the future napi-rs binding surface should the JS library contract return.
//!
//! Phase 0 scaffold — modules land incrementally per the ADR-0003 roadmap:
//!
//! - Phase 1: `tokens`, `ranking` (weighting / boosting / penalties), BM25 math
//! - Phase 2: `chunking` (tree-sitter AST + line fallback)
//! - Phase 3: `indexing` (file walking, dense, sparse, cache)
//! - Phase 4: `search` + the `CspIndex` orchestrator

// pub mod tokens;
// pub mod ranking;
// pub mod chunking;
// pub mod indexing;
// pub mod search;

#[cfg(test)]
mod tests {
    #[test]
    fn scaffold_compiles() {
        assert_eq!(2 + 2, 4);
    }
}
