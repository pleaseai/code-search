//! Misc utilities. Port of `src/utils.ts` (← semble `utils.py`).

use serde_json::{json, Value};

use crate::search::SearchResult;
use crate::types::Chunk;

/// Serialize a search result to the flat CLI/MCP wire dict — **snake_case**
/// fields (`file_path`, `start_line`, `end_line`, `score`, optional `content`),
/// matching semble `utils.format_results` after semble#198.
///
/// `max_snippet_lines` caps the `content` field so agents can spend fewer tokens
/// confirming a location before navigating to the file:
/// - `None` → full chunk content
/// - `Some(0)` → omit `content` entirely (path + line range only)
/// - `Some(n)` → the first `n` lines of content
pub fn result_to_dict(result: &SearchResult, max_snippet_lines: Option<usize>) -> Value {
    let c = &result.chunk;
    let mut entry = json!({
        "file_path": c.file_path,
        "start_line": c.start_line,
        "end_line": c.end_line,
        "score": result.score,
    });
    match max_snippet_lines {
        None => {
            entry["content"] = json!(c.content);
        }
        Some(0) => {}
        Some(n) => {
            let snippet: Vec<&str> = c.content.lines().take(n).collect();
            entry["content"] = json!(snippet.join("\n"));
        }
    }
    entry
}

/// Build the `{ query, results }` payload the CLI prints and the MCP server
/// returns. Port of `utils.format_results`. `max_snippet_lines` is forwarded to
/// [`result_to_dict`] to cap each result's `content`.
pub fn format_results(
    query: &str,
    results: &[SearchResult],
    max_snippet_lines: Option<usize>,
) -> Value {
    json!({
        "query": query,
        "results": results
            .iter()
            .map(|r| result_to_dict(r, max_snippet_lines))
            .collect::<Vec<_>>(),
    })
}

const GIT_URL_SCHEMES: [&str; 6] = [
    "https://",
    "http://",
    "ssh://",
    "git://",
    "git+ssh://",
    "file://",
];

/// Return true if `path` looks like a remote git URL rather than a local path.
pub fn is_git_url(path: &str) -> bool {
    if GIT_URL_SCHEMES
        .iter()
        .any(|scheme| path.starts_with(scheme))
    {
        return true;
    }
    is_scp_git_url(path)
}

/// Reproduce `/^[\w.-]+@[\w.-]+:(?!\/)/`: a scp-style git URL such as
/// `user@host:repo`, but not `user@host:/abs/path`. The negative lookahead is
/// implemented directly (the Rust `regex` crate does not support lookarounds).
fn is_scp_git_url(path: &str) -> bool {
    let b = path.as_bytes();
    let n = b.len();
    let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_' || c == b'.' || c == b'-';

    let mut i = 0;
    // [\w.-]+
    while i < n && is_word(b[i]) {
        i += 1;
    }
    if i == 0 {
        return false;
    }
    // @
    if i >= n || b[i] != b'@' {
        return false;
    }
    i += 1;
    // [\w.-]+
    let host_start = i;
    while i < n && is_word(b[i]) {
        i += 1;
    }
    if i == host_start {
        return false;
    }
    // :
    if i >= n || b[i] != b':' {
        return false;
    }
    i += 1;
    // (?!\/) — the char after ':' must not be a slash (end-of-string is fine).
    !(i < n && b[i] == b'/')
}

/// Return the chunk containing `line` in `file_path`, or `None`.
///
/// A strict inner match (`line < end_line`) wins immediately; a boundary match
/// (`line == end_line`) is kept only as a fallback so end-of-file lines still
/// resolve. Mirrors `semble.utils.resolve_chunk`.
pub fn resolve_chunk<'a>(chunks: &'a [Chunk], file_path: &str, line: u32) -> Option<&'a Chunk> {
    let mut fallback: Option<&Chunk> = None;
    for chunk in chunks {
        if chunk.file_path == file_path && chunk.start_line <= line && line <= chunk.end_line {
            if line < chunk.end_line {
                return Some(chunk);
            }
            if fallback.is_none() {
                fallback = Some(chunk);
            }
        }
    }
    fallback
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(file_path: &str, start_line: u32, end_line: u32) -> Chunk {
        Chunk {
            content: String::new(),
            file_path: file_path.to_string(),
            start_line,
            end_line,
            language: None,
        }
    }

    fn result(content: &str) -> SearchResult {
        SearchResult {
            chunk: Chunk {
                content: content.to_string(),
                file_path: "a.ts".to_string(),
                start_line: 1,
                end_line: 3,
                language: Some("ts".to_string()),
            },
            score: 0.5,
        }
    }

    #[test]
    fn format_results_flat_shape_with_full_content() {
        let out = format_results("q", &[result("line1\nline2\nline3")], None);
        let entry = &out["results"][0];
        // Flat shape: fields live at the top level, no nested `chunk`.
        assert!(entry.get("chunk").is_none());
        assert_eq!(entry["file_path"], "a.ts");
        assert_eq!(entry["start_line"], 1);
        assert_eq!(entry["end_line"], 3);
        assert_eq!(entry["score"], 0.5);
        assert_eq!(entry["content"], "line1\nline2\nline3");
        assert_eq!(out["query"], "q");
    }

    #[test]
    fn result_to_dict_truncates_to_n_lines() {
        let entry = result_to_dict(&result("line1\nline2\nline3\nline4"), Some(2));
        assert_eq!(entry["content"], "line1\nline2");
    }

    #[test]
    fn result_to_dict_omits_content_when_zero() {
        let entry = result_to_dict(&result("line1\nline2"), Some(0));
        assert!(entry.get("content").is_none());
        // Location metadata is still present so the agent can navigate.
        assert_eq!(entry["file_path"], "a.ts");
        assert_eq!(entry["start_line"], 1);
        assert_eq!(entry["end_line"], 3);
    }

    #[test]
    fn result_to_dict_more_lines_than_content_returns_all() {
        let entry = result_to_dict(&result("only\ntwo"), Some(10));
        assert_eq!(entry["content"], "only\ntwo");
    }

    #[test]
    fn recognises_scheme_git_urls() {
        for url in [
            "https://github.com/owner/repo.git",
            "http://example.com/repo",
            "ssh://git@host/repo",
            "git://host/repo",
            "git+ssh://git@host/repo",
            "file:///tmp/repo",
        ] {
            assert!(is_git_url(url), "{url} should be a git url");
        }
    }

    #[test]
    fn recognises_scp_style_git_urls() {
        assert!(is_git_url("git@github.com:owner/repo.git"));
        assert!(is_git_url("user@host:repo"));
    }

    #[test]
    fn rejects_local_paths() {
        assert!(!is_git_url("/abs/path/to/repo"));
        assert!(!is_git_url("./relative/repo"));
        assert!(!is_git_url("repo"));
        // scp form but with an absolute path after `:` is NOT a git url.
        assert!(!is_git_url("user@host:/abs/path"));
    }

    #[test]
    fn resolve_chunk_inner_match_wins() {
        let chunks = [chunk("a.ts", 1, 10), chunk("a.ts", 5, 20)];
        // line 5 is strictly inside the first chunk (5 < 10) → first wins.
        assert_eq!(resolve_chunk(&chunks, "a.ts", 5), Some(&chunks[0]));
    }

    #[test]
    fn resolve_chunk_boundary_is_fallback() {
        let chunks = [chunk("a.ts", 1, 5), chunk("a.ts", 5, 20)];
        // line 5 == end_line of the first (boundary) but strictly inside the
        // second (5 < 20) → the strict inner match wins over the boundary.
        assert_eq!(resolve_chunk(&chunks, "a.ts", 5), Some(&chunks[1]));
    }

    #[test]
    fn resolve_chunk_returns_boundary_when_only_match() {
        let chunks = [chunk("a.ts", 1, 5)];
        assert_eq!(resolve_chunk(&chunks, "a.ts", 5), Some(&chunks[0]));
    }

    #[test]
    fn resolve_chunk_none_when_no_match() {
        let chunks = [chunk("a.ts", 1, 5)];
        assert_eq!(resolve_chunk(&chunks, "b.ts", 3), None);
        assert_eq!(resolve_chunk(&chunks, "a.ts", 99), None);
    }
}
