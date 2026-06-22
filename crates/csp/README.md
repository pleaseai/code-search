# code-search-please (`csp`)

Hybrid code search for agents — the core Rust library behind [`@pleaseai/csp`](https://github.com/pleaseai/code-search). A Rust rewrite of [MinishLab/semble](https://github.com/MinishLab/semble).

> Published to crates.io as **`code-search-please`** (the short name `csp` was taken). The library name is `csp`, so you still write `use csp::...`.

## Install

```toml
[dependencies]
code-search-please = "0.1"
```

## Usage

```rust
use std::path::Path;
use csp::indexing::index::{CspIndex, LoadOptions, QueryOptions};

// Index a local directory and search it
let index = CspIndex::from_path(Path::new("./my-project"), &LoadOptions::default())?;
let results = index.search("save model to disk", &QueryOptions { top_k: Some(3), ..Default::default() });

for r in &results {
    println!("{}:{}-{}", r.chunk.file_path, r.chunk.start_line, r.chunk.end_line);
}
# Ok::<(), String>(())
```

Hybrid scoring combines Model2Vec dense embeddings with BM25, fused via Reciprocal Rank Fusion. Chunking is tree-sitter AST-based with a line-based fallback.

## CLI / MCP

The `csp` binary (CLI + MCP server) ships via npm (`bunx @pleaseai/csp`) and Homebrew. See the [repository README](https://github.com/pleaseai/code-search) for the full surface.

## License

MIT. This is a derivative work of [MinishLab/semble](https://github.com/MinishLab/semble); see the repository for credits and citation.
