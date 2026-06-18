//! `csp` CLI entrypoint. Port of `src/cli.ts`.
//!
//! Wires the clap subcommands to the `csp` core: search / find-related route
//! through the on-disk auto-cache (or an explicit `--index`), index builds and
//! persists, savings/clear drive telemetry, and init writes an agent file.

mod mcp_server;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use csp::indexing::cache::clear_index_cache;
use csp::indexing::index::{
    load_or_build_index, CspIndex, LoadOptions, LoadOrBuildOptions, QueryOptions,
};
use csp::stats::{clear_savings, default_stats_file, format_savings_report, now_secs};
use csp::types::ContentType;
use csp::utils::{format_results, is_git_url, resolve_chunk};

#[derive(Parser)]
#[command(name = "csp", version, about = "Instant local code search for agents")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ContentFilter {
    Code,
    Docs,
    Config,
    All,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Agent {
    Antigravity,
    Claude,
    Commandcode,
    Copilot,
    Cursor,
    Gemini,
    Kiro,
    Opencode,
    Pi,
    Reasonix,
}

#[derive(Subcommand)]
enum Command {
    /// Search for code matching a query.
    Search {
        query: String,
        /// Source path or git URL to index (when --index is omitted).
        path: Option<String>,
        #[arg(long = "top-k", short = 'k')]
        top_k: Option<usize>,
        #[arg(long, value_enum, num_args = 1..)]
        content: Vec<ContentFilter>,
        /// Path to a pre-built index (bypasses the auto-cache).
        #[arg(long)]
        index: Option<String>,
        /// Branch or tag for git URLs.
        #[arg(long = "ref")]
        git_ref: Option<String>,
    },
    /// Find code similar to a specific location.
    #[command(name = "find-related")]
    FindRelated {
        file: String,
        line: String,
        path: Option<String>,
        #[arg(long = "top-k", short = 'k')]
        top_k: Option<usize>,
        #[arg(long, value_enum, num_args = 1..)]
        content: Vec<ContentFilter>,
        #[arg(long)]
        index: Option<String>,
        #[arg(long = "ref")]
        git_ref: Option<String>,
    },
    /// Build a pre-built index and write it to a directory.
    Index {
        path: Option<String>,
        #[arg(long, short = 'o')]
        out: Option<String>,
        #[arg(long, value_enum, num_args = 1..)]
        content: Vec<ContentFilter>,
    },
    /// Run the MCP server (stdio transport).
    Mcp {
        path: Option<String>,
        #[arg(long = "ref")]
        git_ref: Option<String>,
        #[arg(long, value_enum, num_args = 1..)]
        content: Vec<ContentFilter>,
    },
    /// Write a csp sub-agent file for your coding agent.
    Init {
        #[arg(long, short = 'a', value_enum)]
        agent: Option<Agent>,
        #[arg(long)]
        force: bool,
    },
    /// Show token savings and usage stats.
    Savings {
        #[arg(long)]
        verbose: bool,
    },
    /// Clear cached data.
    Clear {
        /// One of: all, index, savings.
        what: String,
    },
}

const CLEAR_CHOICES: &str = "all, index, savings";

impl Agent {
    fn slug(self) -> &'static str {
        match self {
            Agent::Antigravity => "antigravity",
            Agent::Claude => "claude",
            Agent::Commandcode => "commandcode",
            Agent::Copilot => "copilot",
            Agent::Cursor => "cursor",
            Agent::Gemini => "gemini",
            Agent::Kiro => "kiro",
            Agent::Opencode => "opencode",
            Agent::Pi => "pi",
            Agent::Reasonix => "reasonix",
        }
    }

    /// Destination (relative to cwd) of the written sub-agent file.
    fn agent_path(self) -> String {
        let base = if self == Agent::Copilot {
            ".github".to_string()
        } else {
            format!(".{}", self.slug())
        };
        format!("{base}/agents/csp-search.md")
    }

    /// Embedded sub-agent template for this agent.
    fn template(self) -> &'static str {
        match self {
            Agent::Antigravity => include_str!("../agents/antigravity.md"),
            Agent::Claude => include_str!("../agents/claude.md"),
            Agent::Commandcode => include_str!("../agents/commandcode.md"),
            Agent::Copilot => include_str!("../agents/copilot.md"),
            Agent::Cursor => include_str!("../agents/cursor.md"),
            Agent::Gemini => include_str!("../agents/gemini.md"),
            Agent::Kiro => include_str!("../agents/kiro.md"),
            Agent::Opencode => include_str!("../agents/opencode.md"),
            Agent::Pi => include_str!("../agents/pi.md"),
            Agent::Reasonix => include_str!("../agents/reasonix.md"),
        }
    }
}

/// Resolve `--content` flags to content types (empty → code-only; `all` → all).
fn resolve_content(filters: &[ContentFilter]) -> Vec<ContentType> {
    if filters.is_empty() {
        return vec![ContentType::Code];
    }
    if filters.contains(&ContentFilter::All) {
        return vec![ContentType::Code, ContentType::Docs, ContentType::Config];
    }
    let mut out = Vec::new();
    for f in filters {
        let ct = match f {
            ContentFilter::Code => ContentType::Code,
            ContentFilter::Docs => ContentType::Docs,
            ContentFilter::Config => ContentType::Config,
            ContentFilter::All => unreachable!(),
        };
        if !out.contains(&ct) {
            out.push(ct);
        }
    }
    out
}

/// Load the index for a search/find-related call: explicit `--index` loads
/// verbatim; otherwise route through the on-disk auto-cache.
fn load_index(
    index_path: Option<&str>,
    source: &str,
    content: Vec<ContentType>,
    git_ref: Option<String>,
) -> Result<CspIndex, String> {
    if let Some(path) = index_path {
        CspIndex::load_from_disk(Path::new(path))
    } else {
        load_or_build_index(
            source,
            &LoadOrBuildOptions {
                content: Some(content),
                git_ref,
                ..Default::default()
            },
        )
    }
}

/// JSON output for `search` (pure — testable without stdout capture).
fn search_output(index: &CspIndex, query: &str, top_k: usize) -> String {
    let results = index.search(
        query,
        &QueryOptions {
            top_k: Some(top_k),
            ..Default::default()
        },
    );
    let out = if results.is_empty() {
        serde_json::json!({ "error": "No results found." })
    } else {
        format_results(query, &results)
    };
    out.to_string()
}

/// JSON output for `find-related`, or an error message string.
fn find_related_output(
    index: &CspIndex,
    file: &str,
    line: &str,
    top_k: usize,
) -> Result<String, String> {
    let Ok(line_num) = line.parse::<i64>() else {
        return Err(format!("line must be an integer, got: {line}"));
    };
    let chunk = if line_num < 0 {
        None
    } else {
        resolve_chunk(&index.chunks, file, line_num as u32)
    };
    let Some(chunk) = chunk else {
        return Err(format!("No chunk found at {file}:{line_num}."));
    };
    let related = index.find_related(
        &chunk.clone(),
        &QueryOptions {
            top_k: Some(top_k),
            ..Default::default()
        },
    );
    let out = if related.is_empty() {
        serde_json::json!({ "error": format!("No related chunks found for {file}:{line_num}.") })
    } else {
        format_results(&format!("Chunks related to {file}:{line_num}"), &related)
    };
    Ok(out.to_string())
}

/// Write the agent sub-agent file under `cwd`. Returns the relative path written.
fn run_init(agent: Agent, force: bool, cwd: &Path) -> Result<String, String> {
    let rel = agent.agent_path();
    let dest = cwd.join(&rel);
    if dest.exists() && !force {
        return Err(format!(
            "{rel} already exists. Run with --force to overwrite."
        ));
    }
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&dest, agent.template()).map_err(|e| e.to_string())?;
    Ok(rel)
}

fn run_clear(what: &str) -> ExitCode {
    if !["all", "index", "savings"].contains(&what) {
        eprintln!("Invalid clear type: {what}. Choices: {CLEAR_CHOICES}");
        return ExitCode::FAILURE;
    }
    if what == "index" || what == "all" {
        match clear_index_cache(&Default::default()) {
            Ok(r) if r.cleared => {
                println!(
                    "Cleared {} cached index entries at `{}`",
                    r.entries,
                    r.path.display()
                );
            }
            Ok(r) => println!("No index cache found at `{}`", r.path.display()),
            Err(e) => eprintln!("{e}"),
        }
    }
    if what == "savings" || what == "all" {
        let (path, cleared) = clear_savings(&default_stats_file());
        if cleared {
            println!("Cleared savings at `{}`", path.display());
        } else {
            println!("No savings file found at `{}`", path.display());
        }
    }
    ExitCode::SUCCESS
}

fn run() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Init { agent, force } => {
            let agent = agent.unwrap_or(Agent::Claude);
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            match run_init(agent, force, &cwd) {
                Ok(rel) => {
                    println!("Created {rel}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Savings { verbose } => {
            print!(
                "{}",
                format_savings_report(&default_stats_file(), verbose, now_secs())
            );
            ExitCode::SUCCESS
        }
        Command::Clear { what } => run_clear(&what),
        Command::Index { path, out, content } => {
            let Some(out) = out else {
                eprintln!("--out / -o is required for `index`.");
                return ExitCode::FAILURE;
            };
            let path = path.unwrap_or_else(|| ".".to_string());
            let options = LoadOptions {
                content: Some(resolve_content(&content)),
                ..Default::default()
            };
            let built = if is_git_url(&path) {
                CspIndex::from_git(&path, &options, None)
            } else {
                CspIndex::from_path(Path::new(&path), &options)
            };
            match built.and_then(|idx| idx.save(Path::new(&out), None)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Search {
            query,
            path,
            top_k,
            content,
            index,
            git_ref,
        } => {
            let source = path.unwrap_or_else(|| ".".to_string());
            match load_index(
                index.as_deref(),
                &source,
                resolve_content(&content),
                git_ref,
            ) {
                Ok(idx) => {
                    println!("{}", search_output(&idx, &query, top_k.unwrap_or(5)));
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::FindRelated {
            file,
            line,
            path,
            top_k,
            content,
            index,
            git_ref,
        } => {
            let source = path.unwrap_or_else(|| ".".to_string());
            let idx = match load_index(
                index.as_deref(),
                &source,
                resolve_content(&content),
                git_ref,
            ) {
                Ok(idx) => idx,
                Err(e) => {
                    eprintln!("{e}");
                    return ExitCode::FAILURE;
                }
            };
            match find_related_output(&idx, &file, &line, top_k.unwrap_or(5)) {
                Ok(out) => {
                    println!("{out}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
        Command::Mcp { path, content, .. } => {
            // `path` is the default source for tool calls that omit `repo`;
            // None when no path was given (the tool then requires an explicit `repo`).
            match mcp_server::run_mcp(path, resolve_content(&content)) {
                Ok(()) => ExitCode::SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    ExitCode::FAILURE
                }
            }
        }
    }
}

fn main() -> ExitCode {
    run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn resolve_content_defaults_to_code() {
        assert_eq!(resolve_content(&[]), vec![ContentType::Code]);
    }

    #[test]
    fn resolve_content_all_expands() {
        assert_eq!(
            resolve_content(&[ContentFilter::All]),
            vec![ContentType::Code, ContentType::Docs, ContentType::Config]
        );
    }

    #[test]
    fn resolve_content_dedups() {
        assert_eq!(
            resolve_content(&[ContentFilter::Docs, ContentFilter::Docs]),
            vec![ContentType::Docs]
        );
    }

    #[test]
    fn agent_path_uses_github_for_copilot() {
        assert_eq!(Agent::Copilot.agent_path(), ".github/agents/csp-search.md");
        assert_eq!(Agent::Claude.agent_path(), ".claude/agents/csp-search.md");
    }

    #[test]
    fn init_writes_then_guards_overwrite() {
        let dir = tempdir().unwrap();
        let rel = run_init(Agent::Claude, false, dir.path()).unwrap();
        assert_eq!(rel, ".claude/agents/csp-search.md");
        let written = dir.path().join(&rel);
        assert!(written.exists());
        assert!(!std::fs::read_to_string(&written).unwrap().is_empty());

        let err = run_init(Agent::Claude, false, dir.path()).unwrap_err();
        assert!(err.contains("already exists"));
        assert!(run_init(Agent::Claude, true, dir.path()).is_ok());
    }

    fn build_index_dir() -> tempfile::TempDir {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("sample.ts"),
            "export function greet(name: string) { return `hi ${name}` }\n",
        )
        .unwrap();
        dir
    }

    #[test]
    fn search_output_shapes_results() {
        let dir = build_index_dir();
        let idx = CspIndex::from_path(dir.path(), &LoadOptions::default()).unwrap();
        let out = search_output(&idx, "greet", 5);
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(value.get("results").is_some() || value.get("error").is_some());
        if let Some(results) = value.get("results").and_then(|r| r.as_array()) {
            if let Some(first) = results.first() {
                let chunk = &first["chunk"];
                assert!(chunk.get("file_path").is_some());
                assert!(chunk.get("start_line").is_some());
                assert!(chunk.get("location").is_some());
            }
        }
    }

    #[test]
    fn find_related_rejects_non_integer_line() {
        let dir = build_index_dir();
        let idx = CspIndex::from_path(dir.path(), &LoadOptions::default()).unwrap();
        let err = find_related_output(&idx, "sample.ts", "abc", 5).unwrap_err();
        assert!(err.contains("line must be an integer"));
    }

    #[test]
    fn find_related_no_chunk_at_location() {
        let dir = build_index_dir();
        let idx = CspIndex::from_path(dir.path(), &LoadOptions::default()).unwrap();
        let err = find_related_output(&idx, "nope.ts", "1", 5).unwrap_err();
        assert!(err.contains("No chunk found"));
    }
}
