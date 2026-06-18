//! Chunking. Port of `src/chunking/*` (← semble `chunking/`).
//!
//! `core` holds the AST/line chunking algorithm (generic over [`core::AstNode`]);
//! `source` is the public entry point producing [`crate::types::Chunk`] values.

pub mod core;
pub mod source;
