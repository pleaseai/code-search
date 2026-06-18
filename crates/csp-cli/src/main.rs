//! `csp` CLI entrypoint — Phase 0 scaffold (ADR-0003).
//!
//! Subcommands mirror the README surface and the TypeScript implementation.
//! Each is stubbed until its migration phase lands (CLI wiring is Phase 5).

use clap::{Parser, Subcommand, ValueEnum};

#[derive(Parser)]
#[command(name = "csp", version, about = "Hybrid code search for agents")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum ContentFilter {
    Code,
    Docs,
    Config,
    All,
}

#[derive(Subcommand)]
enum Command {
    /// Search an index for code matching a query.
    Search {
        /// The search query.
        query: String,
        /// Maximum number of results to return.
        #[arg(long = "top-k")]
        top_k: Option<usize>,
        /// Restrict results to a content type.
        #[arg(long, value_enum)]
        content: Option<ContentFilter>,
        /// Path to the index.
        #[arg(long)]
        index: Option<String>,
    },
    /// Build or refresh the index for a path.
    Index {
        /// Path to index (defaults to the current directory).
        path: Option<String>,
    },
    /// Find chunks related to a file or symbol.
    #[command(name = "find-related")]
    FindRelated {
        /// File path or symbol to find relations for.
        target: String,
    },
    /// Run the MCP server (stdio transport).
    Mcp,
    /// Initialize csp for an agent.
    Init {
        /// Target agent.
        #[arg(long)]
        agent: Option<String>,
    },
    /// Show token-savings statistics.
    Savings,
    /// Clear cached data (e.g. the global index cache).
    Clear {
        /// What to clear (e.g. `index`).
        what: String,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Search { .. } => not_yet("search"),
        Command::Index { .. } => not_yet("index"),
        Command::FindRelated { .. } => not_yet("find-related"),
        Command::Mcp => not_yet("mcp"),
        Command::Init { .. } => not_yet("init"),
        Command::Savings => not_yet("savings"),
        Command::Clear { .. } => not_yet("clear"),
    }
}

fn not_yet(name: &str) -> anyhow::Result<()> {
    anyhow::bail!("`csp {name}` is not implemented yet (Rust rewrite in progress — see ADR-0003)")
}
