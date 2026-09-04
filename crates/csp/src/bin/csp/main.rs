//! `csp` CLI entrypoint. Port of `src/cli.ts`.
//!
//! Wires the clap subcommands to the `csp` core: search / find-related route
//! through the on-disk auto-cache (or an explicit `--index`), index builds and
//! persists, savings/clear drive telemetry, and init writes an agent file.

mod mcp_server;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use csp::indexing::cache::{clear_index_cache, clear_orphan_indexes, CacheLocation};
use csp::indexing::index::{
    load_or_build_index, CspIndex, LoadOptions, LoadOrBuildOptions, QueryOptions,
};
use csp::stats::{
    clear_savings, default_stats_file, format_savings_report, now_secs, save_search_stats,
};
use csp::types::{CallType, ContentType};
use csp::utils::{format_results, is_git_url, resolve_chunk, resolve_snippet_lines};

#[derive(Parser)]
#[command(name = "csp", version, about = "Instant local code search for agents")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
enum ContentFilter {
    Code,
    Docs,
    Config,
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
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
        /// Lines of source per result (default: full chunk). 10 = signature +
        /// body, 0 = no code.
        #[arg(long = "max-snippet-lines", value_name = "N")]
        max_snippet_lines: Option<i64>,
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
        /// Lines of source per result (default: full chunk). 10 = signature +
        /// body, 0 = no code.
        #[arg(long = "max-snippet-lines", value_name = "N")]
        max_snippet_lines: Option<i64>,
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
        /// One of: all, index, savings, orphans. `orphans` removes cached
        /// indexes whose source path no longer exists (not part of `all`).
        what: String,
    },
}

const CLEAR_CHOICES: &str = "all, index, savings, orphans";

/// Process exit codes returned by `dispatch` / `run_clear` (mapped to
/// `ExitCode` in `run`). Plain `u8` so tests can assert on them directly.
const EXIT_SUCCESS: u8 = 0;
const EXIT_FAILURE: u8 = 1;

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
            Agent::Antigravity => include_str!("agents/antigravity.md"),
            Agent::Claude => include_str!("agents/claude.md"),
            Agent::Commandcode => include_str!("agents/commandcode.md"),
            Agent::Copilot => include_str!("agents/copilot.md"),
            Agent::Cursor => include_str!("agents/cursor.md"),
            Agent::Gemini => include_str!("agents/gemini.md"),
            Agent::Kiro => include_str!("agents/kiro.md"),
            Agent::Opencode => include_str!("agents/opencode.md"),
            Agent::Pi => include_str!("agents/pi.md"),
            Agent::Reasonix => include_str!("agents/reasonix.md"),
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

/// JSON output for `search`. `stats_file` records token-savings telemetry when
/// `Some`; tests pass `None` to stay off the real `~/.csp` file.
fn search_output(
    index: &CspIndex,
    query: &str,
    top_k: usize,
    max_snippet_lines: Option<usize>,
    stats_file: Option<&Path>,
) -> String {
    let results = index.search(
        query,
        &QueryOptions {
            top_k: Some(top_k),
            ..Default::default()
        },
    );
    if let Some(stats_file) = stats_file {
        save_search_stats(
            stats_file,
            &results,
            CallType::Search,
            &index.file_sizes,
            max_snippet_lines,
        );
    }
    let out = if results.is_empty() {
        serde_json::json!({ "error": "No results found." })
    } else {
        format_results(query, &results, max_snippet_lines)
    };
    out.to_string()
}

/// JSON output for `find-related`, or an error message string.
fn find_related_output(
    index: &CspIndex,
    file: &str,
    line: &str,
    top_k: usize,
    max_snippet_lines: Option<usize>,
    stats_file: Option<&Path>,
) -> Result<String, String> {
    let Ok(line_num) = line.parse::<i64>() else {
        return Err(format!("line must be an integer, got: {line}"));
    };
    // Guard the full u32 range, not just the lower bound — a line number above
    // u32::MAX would otherwise wrap on `as u32` and resolve the wrong chunk.
    let chunk = if (0..=i64::from(u32::MAX)).contains(&line_num) {
        resolve_chunk(&index.chunks, file, line_num as u32)
    } else {
        None
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
    if let Some(stats_file) = stats_file {
        save_search_stats(
            stats_file,
            &related,
            CallType::FindRelated,
            &index.file_sizes,
            max_snippet_lines,
        );
    }
    let out = if related.is_empty() {
        serde_json::json!({ "error": format!("No related chunks found for {file}:{line_num}.") })
    } else {
        format_results(
            &format!("Chunks related to {file}:{line_num}"),
            &related,
            max_snippet_lines,
        )
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

fn run_clear(what: &str) -> u8 {
    run_clear_at(what, &Default::default(), &default_stats_file())
}

/// `run_clear` with the cache location and savings file injected, so tests can
/// exercise the destructive branches against temp dirs instead of real `~/.csp`.
fn run_clear_at(what: &str, cache_loc: &CacheLocation, stats_file: &Path) -> u8 {
    if !["all", "index", "savings", "orphans"].contains(&what) {
        eprintln!("Invalid clear type: {what}. Choices: {CLEAR_CHOICES}");
        return EXIT_FAILURE;
    }
    // Track failures so a maintenance command that couldn't clear the index
    // reports a non-zero exit status (automation relies on it).
    let mut failed = false;
    if what == "index" || what == "all" {
        match clear_index_cache(cache_loc) {
            Ok(r) if r.cleared => {
                println!(
                    "Cleared {} cached index entries at `{}`",
                    r.entries,
                    r.path.display()
                );
            }
            Ok(r) => println!("No index cache found at `{}`", r.path.display()),
            Err(e) => {
                eprintln!("{e}");
                failed = true;
            }
        }
    }
    if what == "savings" || what == "all" {
        let (path, cleared) = clear_savings(stats_file);
        if cleared {
            println!("Cleared savings at `{}`", path.display());
        } else {
            println!("No savings file found at `{}`", path.display());
        }
    }
    // Mirrors upstream: `orphans` is its own choice, never folded into `all`.
    if what == "orphans" {
        match clear_orphan_indexes(cache_loc) {
            Ok(removed) if removed.is_empty() => println!("No orphaned indexes found"),
            Ok(removed) => {
                for orphan in removed {
                    println!("Cleared orphaned index for `{}`", orphan.source_id);
                }
            }
            Err(e) => {
                eprintln!("{e}");
                failed = true;
            }
        }
    }
    if failed {
        EXIT_FAILURE
    } else {
        EXIT_SUCCESS
    }
}

fn run() -> ExitCode {
    ExitCode::from(dispatch(Cli::parse().command))
}

/// Execute a parsed subcommand, returning a process exit code (`0` success,
/// `1` failure). Split from `run` — and returning a plain `u8` rather than
/// `ExitCode` — so the dispatch logic is unit-testable without going through
/// `Cli::parse` (which reads argv) or an opaque, non-comparable `ExitCode`.
fn dispatch(command: Command) -> u8 {
    dispatch_with_stats(command, &default_stats_file())
}

/// [`dispatch`] with the token-savings file injected so tests can redirect the
/// telemetry that `search` / `find-related` append (keeping the real `~/.csp`
/// untouched).
fn dispatch_with_stats(command: Command, stats_file: &Path) -> u8 {
    match command {
        Command::Init { agent, force } => {
            let agent = agent.unwrap_or(Agent::Claude);
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            match run_init(agent, force, &cwd) {
                Ok(rel) => {
                    println!("Created {rel}");
                    EXIT_SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    EXIT_FAILURE
                }
            }
        }
        Command::Savings { verbose } => {
            print!(
                "{}",
                format_savings_report(&default_stats_file(), verbose, now_secs())
            );
            EXIT_SUCCESS
        }
        Command::Clear { what } => run_clear(&what),
        Command::Index { path, out, content } => {
            let Some(out) = out else {
                eprintln!("--out / -o is required for `index`.");
                return EXIT_FAILURE;
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
                Ok(()) => EXIT_SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    EXIT_FAILURE
                }
            }
        }
        Command::Search {
            query,
            path,
            top_k,
            max_snippet_lines,
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
                    println!(
                        "{}",
                        search_output(
                            &idx,
                            &query,
                            top_k.unwrap_or(5),
                            resolve_snippet_lines(max_snippet_lines),
                            Some(stats_file),
                        )
                    );
                    EXIT_SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    EXIT_FAILURE
                }
            }
        }
        Command::FindRelated {
            file,
            line,
            path,
            top_k,
            max_snippet_lines,
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
                    return EXIT_FAILURE;
                }
            };
            match find_related_output(
                &idx,
                &file,
                &line,
                top_k.unwrap_or(5),
                resolve_snippet_lines(max_snippet_lines),
                Some(stats_file),
            ) {
                Ok(out) => {
                    println!("{out}");
                    EXIT_SUCCESS
                }
                Err(e) => {
                    eprintln!("{e}");
                    EXIT_FAILURE
                }
            }
        }
        Command::Mcp {
            path,
            git_ref,
            content,
        } => {
            // `path` is the default source for tool calls that omit `repo`;
            // None when no path was given (the tool then requires an explicit `repo`).
            // `git_ref` (--ref) pins the revision when that default source is a git URL.
            match mcp_server::run_mcp(path, git_ref, resolve_content(&content)) {
                Ok(()) => EXIT_SUCCESS,
                Err(e) => {
                    eprintln!("{e}");
                    EXIT_FAILURE
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
        let out = search_output(&idx, "greet", 5, None, None);
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert!(value.get("results").is_some() || value.get("error").is_some());
        if let Some(results) = value.get("results").and_then(|r| r.as_array()) {
            if let Some(first) = results.first() {
                // Flat wire shape (semble#198): fields at the top level.
                assert!(first.get("chunk").is_none());
                assert!(first.get("file_path").is_some());
                assert!(first.get("start_line").is_some());
                // CLI default (None) keeps the full content.
                assert!(first.get("content").is_some());
            }
        }
    }

    #[test]
    fn search_output_caps_snippet_lines() {
        let dir = build_index_dir();
        let idx = CspIndex::from_path(dir.path(), &LoadOptions::default()).unwrap();
        let out = search_output(&idx, "greet", 5, Some(0), None);
        let value: serde_json::Value = serde_json::from_str(&out).unwrap();
        if let Some(first) = value["results"].as_array().and_then(|r| r.first()) {
            assert!(first.get("content").is_none());
            assert!(first.get("file_path").is_some());
        }
    }

    #[test]
    fn search_output_records_savings_when_stats_file_given() {
        let dir = build_index_dir();
        let idx = CspIndex::from_path(dir.path(), &LoadOptions::default()).unwrap();
        // The source tree is still on disk, so sizes are read lazily per result.
        // Look the size up under the path the chunks actually carry — that is
        // the key `save_search_stats` will use.
        let indexed_path = idx.chunks[0].file_path.clone();
        assert!(idx.file_sizes.get(&indexed_path).is_some());

        let stats = tempdir().unwrap();
        let stats_file = stats.path().join("savings.jsonl");
        let _ = search_output(&idx, "greet", 5, None, Some(&stats_file));

        let content = std::fs::read_to_string(&stats_file).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 1);
        let record: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(record["call"], "search");
        // A nonzero value, not just the key: a lazy lookup that resolved nothing
        // would still serialize `"file_chars":0`.
        assert!(record["file_chars"].as_u64().unwrap() > 0);
        assert!(record["snippet_chars"].as_u64().unwrap() > 0);
    }

    #[test]
    fn find_related_rejects_non_integer_line() {
        let dir = build_index_dir();
        let idx = CspIndex::from_path(dir.path(), &LoadOptions::default()).unwrap();
        let err = find_related_output(&idx, "sample.ts", "abc", 5, None, None).unwrap_err();
        assert!(err.contains("line must be an integer"));
    }

    #[test]
    fn find_related_no_chunk_at_location() {
        let dir = build_index_dir();
        let idx = CspIndex::from_path(dir.path(), &LoadOptions::default()).unwrap();
        let err = find_related_output(&idx, "nope.ts", "1", 5, None, None).unwrap_err();
        assert!(err.contains("No chunk found"));
    }

    #[test]
    fn run_clear_rejects_unknown_type() {
        // Validation-only branch — does not touch any real ~/.csp data.
        assert_eq!(run_clear("bogus"), EXIT_FAILURE);
    }

    #[test]
    fn run_clear_at_handles_index_savings_and_all() {
        // Point both the cache home and the savings file at temp dirs so the
        // destructive branches run without touching real ~/.csp.
        let home = tempdir().unwrap();
        let loc = CacheLocation {
            base_dir: Some(home.path().to_path_buf()),
            ..Default::default()
        };
        let stats = home.path().join("savings.jsonl");

        // Nothing there yet → still a clean (success) exit for each branch.
        assert_eq!(run_clear_at("index", &loc, &stats), EXIT_SUCCESS);
        assert_eq!(run_clear_at("savings", &loc, &stats), EXIT_SUCCESS);
        assert_eq!(run_clear_at("all", &loc, &stats), EXIT_SUCCESS);
        assert_eq!(run_clear_at("orphans", &loc, &stats), EXIT_SUCCESS);
    }

    #[test]
    fn run_clear_at_orphans_removes_only_dead_sources() {
        use csp::indexing::cache::{resolve_cache_dir, resolve_index_root};
        use csp::indexing::index::{IndexManifest, INDEX_SCHEMA_VERSION};

        let home = tempdir().unwrap();
        let loc = CacheLocation {
            base_dir: Some(home.path().join(".csp")),
            ..Default::default()
        };
        let stats = home.path().join("savings.jsonl");
        let write_entry = |source: &Path| -> PathBuf {
            let dir = resolve_cache_dir(&source.to_string_lossy(), &[ContentType::Code], &loc);
            std::fs::create_dir_all(&dir).unwrap();
            let manifest = IndexManifest {
                schema_version: INDEX_SCHEMA_VERSION,
                content_hash: "hash".to_string(),
                source_id: Some(source.to_string_lossy().into_owned()),
                content: vec![ContentType::Code],
                model_id: "model".to_string(),
                model_kind: Some("stub".to_string()),
                chunk_size: Some(750),
                files: Default::default(),
            };
            std::fs::write(
                dir.join("manifest.json"),
                serde_json::to_string(&manifest).unwrap(),
            )
            .unwrap();
            dir
        };
        let live = home.path().join("live");
        std::fs::create_dir_all(&live).unwrap();
        let live_dir = write_entry(&live);
        let dead_dir = write_entry(&home.path().join("gone"));

        assert_eq!(run_clear_at("orphans", &loc, &stats), EXIT_SUCCESS);
        assert!(live_dir.exists());
        assert!(!dead_dir.exists());
        // `all` never sweeps orphans on its own; it removes the whole root,
        // live entries included.
        assert_eq!(run_clear_at("all", &loc, &stats), EXIT_SUCCESS);
        assert!(!live_dir.exists());
        assert!(!resolve_index_root(&loc).exists());
    }

    #[test]
    fn dispatch_savings_succeeds() {
        // format_savings_report only reads (a possibly-absent) savings file.
        assert_eq!(dispatch(Command::Savings { verbose: true }), EXIT_SUCCESS);
    }

    /// Build a pre-built index into `out_dir` via the `index` subcommand.
    fn index_to(out_dir: &Path, src_dir: &Path) -> u8 {
        dispatch(Command::Index {
            path: Some(src_dir.to_string_lossy().into_owned()),
            out: Some(out_dir.to_string_lossy().into_owned()),
            content: vec![],
        })
    }

    #[test]
    fn dispatch_index_requires_out() {
        let src = build_index_dir();
        let code = dispatch(Command::Index {
            path: Some(src.path().to_string_lossy().into_owned()),
            out: None,
            content: vec![],
        });
        assert_eq!(code, EXIT_FAILURE);
    }

    #[test]
    fn dispatch_index_then_search_and_find_related() {
        // Keep everything on an explicit --index path so the test never writes
        // to the global ~/.csp auto-cache, and redirect savings telemetry to a
        // temp file so it never touches the real ~/.csp/savings.jsonl.
        let src = build_index_dir();
        let out = tempdir().unwrap();
        let stats_file = out.path().join("savings.jsonl");

        assert_eq!(index_to(out.path(), src.path()), EXIT_SUCCESS);

        let idx_path = out.path().to_string_lossy().into_owned();
        let search = dispatch_with_stats(
            Command::Search {
                query: "greet".to_string(),
                path: None,
                top_k: Some(5),
                max_snippet_lines: None,
                content: vec![],
                index: Some(idx_path.clone()),
                git_ref: None,
            },
            &stats_file,
        );
        assert_eq!(search, EXIT_SUCCESS);

        // sample.ts:1 has an indexable chunk → find-related succeeds.
        let related = dispatch_with_stats(
            Command::FindRelated {
                file: "sample.ts".to_string(),
                line: "1".to_string(),
                path: None,
                top_k: Some(5),
                max_snippet_lines: None,
                content: vec![],
                index: Some(idx_path.clone()),
                git_ref: None,
            },
            &stats_file,
        );
        assert_eq!(related, EXIT_SUCCESS);

        // A non-integer line is a caller error → failure exit.
        let bad = dispatch_with_stats(
            Command::FindRelated {
                file: "sample.ts".to_string(),
                line: "abc".to_string(),
                path: None,
                top_k: Some(5),
                max_snippet_lines: None,
                content: vec![],
                index: Some(idx_path),
                git_ref: None,
            },
            &stats_file,
        );
        assert_eq!(bad, EXIT_FAILURE);

        // The two successful calls appended one savings record each.
        let recorded = std::fs::read_to_string(&stats_file).unwrap();
        assert_eq!(recorded.lines().filter(|l| !l.is_empty()).count(), 2);
    }

    #[test]
    fn dispatch_search_reports_missing_index() {
        // A nonexistent explicit --index path surfaces a load error → failure.
        let missing = tempdir().unwrap();
        let code = dispatch(Command::Search {
            query: "greet".to_string(),
            path: None,
            top_k: Some(1),
            max_snippet_lines: None,
            content: vec![],
            index: Some(
                missing
                    .path()
                    .join("does-not-exist")
                    .to_string_lossy()
                    .into_owned(),
            ),
            git_ref: None,
        });
        assert_eq!(code, EXIT_FAILURE);
    }

    #[test]
    fn dispatch_init_writes_agent_file() {
        // Init writes under the current working dir; run it in a temp cwd so it
        // does not pollute the repo. Serialize cwd mutation with a mutex since
        // tests share the process.
        use std::sync::Mutex;
        static CWD_LOCK: Mutex<()> = Mutex::new(());
        let _guard = CWD_LOCK.lock().unwrap();

        let dir = tempdir().unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(dir.path()).unwrap();
        let code = dispatch(Command::Init {
            agent: Some(Agent::Claude),
            force: false,
        });
        std::env::set_current_dir(original).unwrap();

        assert_eq!(code, EXIT_SUCCESS);
        assert!(dir.path().join(".claude/agents/csp-search.md").exists());
    }
}
