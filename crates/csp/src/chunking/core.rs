//! AST-based chunker with a line-based fallback. Port of
//! `src/chunking/core.ts` (← semble `chunking/core.py`).
//!
//! The merge algorithm is generic over [`AstNode`] so it can be unit-tested with
//! mock nodes; in production it is driven by [`tree_sitter::Node`] via [`TsNode`].
//! Grammars come from [`tree_sitter_language_pack`] (306 languages, full upstream
//! parity — semble uses the Python `tree_sitter_language_pack`; see ADR-0004).
//! Parsers are fetched from GitHub releases on first use and cached on disk; a
//! language with no available grammar — or an offline fetch failure — makes
//! [`language_for`] return `None`, so [`chunk`] returns `None` and callers fall
//! back to [`chunk_lines`], exactly the upstream behavior when the language pack
//! has no parser for the language.

use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

use tree_sitter::{Language, Parser};

pub const RECURSION_DEPTH: usize = 500;
pub const MIN_CHUNK_SIZE: usize = 50;

/// Languages we've already warned about failing to resolve, so a polyglot
/// offline index degrades quietly after the first notice per language.
static WARNED_LANGUAGES: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

/// Resolve a semble language name (the values in
/// [`crate::indexing::files`]'s `EXTENSION_TO_LANGUAGE`) to a tree-sitter grammar
/// from the language pack, or `None` when the pack has no grammar for it.
///
/// This calls [`tree_sitter_language_pack::get_language`], which downloads the
/// parser from GitHub releases on first use and caches it on disk; later calls
/// hit the in-process registry. A network failure (offline) or an unknown
/// language degrades to `None` → line chunking, with a one-time stderr warning
/// per language so the cause is visible without spamming a polyglot index. Use
/// [`is_supported_language`] (a metadata-only check) when you only need to know
/// whether a grammar *exists* without triggering a download.
pub fn language_for(language: &str) -> Option<Language> {
    match tree_sitter_language_pack::get_language(language) {
        Ok(lang) => Some(lang),
        Err(err) => {
            // Don't swallow the error silently: warn once per language, then
            // fall back to line chunking (the caller treats `None` that way).
            if let Ok(mut warned) = WARNED_LANGUAGES.lock() {
                if warned.insert(language.to_string()) {
                    eprintln!(
                        "csp: could not load tree-sitter grammar for '{language}': {err}. \
                         Falling back to line-based chunking for this language."
                    );
                }
            }
            None
        }
    }
}

/// A half-open `[start, end)` boundary in character offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkBoundary {
    pub start: usize,
    pub end: usize,
}

/// The minimal structural shape of a tree-sitter node the chunker depends on.
pub trait AstNode: Sized {
    fn start_byte(&self) -> usize;
    fn end_byte(&self) -> usize;
    fn children(&self) -> Vec<Self>;
}

/// Check whether the language pack knows a grammar for `language`.
///
/// This is a metadata-only lookup ([`tree_sitter_language_pack::has_language`],
/// resolving aliases) — unlike [`language_for`] it does **not** download the
/// parser, so [`crate::chunking::source::chunk_source`] can gate AST chunking
/// cheaply before paying for a fetch. A recognized language can still fall back
/// to line chunking later if the actual download fails (e.g. offline).
pub fn is_supported_language(language: &str) -> bool {
    tree_sitter_language_pack::has_language(language)
}

/// [`AstNode`] adapter over a real [`tree_sitter::Node`]. `Node` is `Copy` and
/// carries the tree's lifetime, so children are collected via a transient cursor.
struct TsNode<'tree>(tree_sitter::Node<'tree>);

impl<'tree> AstNode for TsNode<'tree> {
    fn start_byte(&self) -> usize {
        self.0.start_byte()
    }
    fn end_byte(&self) -> usize {
        self.0.end_byte()
    }
    fn children(&self) -> Vec<Self> {
        let mut cursor = self.0.walk();
        self.0.children(&mut cursor).map(TsNode).collect()
    }
}

/// Convert a UTF-8 byte offset into a character offset (Python parity:
/// `len(as_bytes[:offset].decode("utf-8"))`). Floors to the nearest char
/// boundary so a mid-codepoint offset can't panic.
fn byte_to_char(text: &str, byte: usize) -> usize {
    let mut b = byte.min(text.len());
    while b > 0 && !text.is_char_boundary(b) {
        b -= 1;
    }
    text[..b].chars().count()
}

/// Merge adjacent chunks up to the desired length.
pub fn merge_adjacent_chunks(
    chunks: &[ChunkBoundary],
    desired_length: usize,
) -> Vec<ChunkBoundary> {
    if chunks.is_empty() {
        return Vec::new();
    }

    let mut merged = Vec::new();
    let first = chunks[0];
    let mut current_start = first.start;
    let mut current_end = first.end;
    let mut current_length = current_end.saturating_sub(current_start);

    for group in &chunks[1..] {
        let length = group.end.saturating_sub(group.start);
        if current_length + length > desired_length {
            merged.push(ChunkBoundary {
                start: current_start,
                end: current_end,
            });
            current_start = group.start;
            current_end = group.end;
            current_length = length;
            continue;
        }
        current_end = group.end;
        current_length += length;
    }

    merged.push(ChunkBoundary {
        start: current_start,
        end: current_end,
    });
    merged
}

/// Recursively merge and split nodes.
pub fn merge_node_inner<N: AstNode>(
    node: &N,
    desired_length: usize,
    depth: usize,
) -> Vec<ChunkBoundary> {
    let children = node.children();

    let whole = ChunkBoundary {
        start: node.start_byte(),
        end: node.end_byte(),
    };

    // No children → only option is the node itself.
    if children.is_empty() {
        return vec![whole];
    }
    let length = node.end_byte().saturating_sub(node.start_byte());
    // Guard pathological recursion, and don't recurse into short nodes.
    if depth > RECURSION_DEPTH || length < MIN_CHUNK_SIZE {
        return vec![whole];
    }

    let mut groups = Vec::new();
    let mut index = 0;
    while index < children.len() {
        let child = &children[index];
        let start = child.start_byte();
        let mut end = child.end_byte();
        let mut run_length = end.saturating_sub(start);
        index += 1;

        // A single oversized child is split further.
        if run_length > desired_length {
            groups.extend(merge_node_inner(child, desired_length, depth + 1));
            continue;
        }

        // Extend the group with following children while they fit.
        while index < children.len() {
            let next = &children[index];
            let child_length = next.end_byte().saturating_sub(next.start_byte());
            if run_length + child_length > desired_length {
                break;
            }
            end = next.end_byte();
            run_length += child_length;
            index += 1;
        }

        groups.push(ChunkBoundary { start, end });
    }

    groups
}

/// Recursively turn nodes into chunks, then merge adjacent chunks.
pub fn merge_node<N: AstNode>(node: &N, desired_length: usize) -> Vec<ChunkBoundary> {
    let raw = merge_node_inner(node, desired_length, 0);
    merge_adjacent_chunks(&raw, desired_length)
}

/// Split `text` into lines preserving the trailing newline on each line —
/// equivalent to Python's `str.splitlines(keepends=True)` for `\n`, `\r\n`,
/// and bare `\r`.
fn split_lines_keep_ends(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let bytes = text.as_bytes();
    let n = bytes.len();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < n {
        match bytes[i] {
            b'\n' => {
                lines.push(&text[start..i + 1]);
                i += 1;
                start = i;
            }
            b'\r' => {
                if i + 1 < n && bytes[i + 1] == b'\n' {
                    lines.push(&text[start..i + 2]);
                    i += 2;
                } else {
                    lines.push(&text[start..i + 1]);
                    i += 1;
                }
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < n {
        lines.push(&text[start..]);
    }
    lines
}

/// Chunk source code by line (character offsets).
pub fn chunk_lines(text: &str, desired_length: usize) -> Vec<ChunkBoundary> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let mut lines_as_groups = Vec::new();
    let mut index = 0;
    for line in split_lines_keep_ends(text) {
        let len = line.chars().count();
        lines_as_groups.push(ChunkBoundary {
            start: index,
            end: index + len,
        });
        index += len;
    }
    merge_adjacent_chunks(&lines_as_groups, desired_length)
}

/// Chunk source via tree-sitter. Returns `Some(vec![])` for whitespace-only
/// input, and `None` when no grammar is bundled for `language` or parsing fails
/// (callers fall back to [`chunk_lines`]).
///
/// The merge runs on tree-sitter **byte** offsets (as upstream does), then each
/// boundary is converted to a **character** offset for the caller — matching
/// semble's `len(as_bytes[:n].decode("utf-8"))`.
pub fn chunk(text: &str, language: &str, desired_length: usize) -> Option<Vec<ChunkBoundary>> {
    if text.trim().is_empty() {
        return Some(Vec::new());
    }
    let lang = language_for(language)?;
    let mut parser = Parser::new();
    if parser.set_language(&lang).is_err() {
        return None;
    }
    let tree = parser.parse(text.as_bytes(), None)?;
    let byte_boundaries = merge_node(&TsNode(tree.root_node()), desired_length);
    Some(
        byte_boundaries
            .iter()
            .map(|b| ChunkBoundary {
                start: byte_to_char(text, b.start),
                end: byte_to_char(text, b.end),
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone)]
    struct FakeNode {
        start: usize,
        end: usize,
        children: Vec<FakeNode>,
    }

    impl AstNode for FakeNode {
        fn start_byte(&self) -> usize {
            self.start
        }
        fn end_byte(&self) -> usize {
            self.end
        }
        fn children(&self) -> Vec<Self> {
            self.children.clone()
        }
    }

    fn leaf(start: usize, end: usize) -> FakeNode {
        FakeNode {
            start,
            end,
            children: vec![],
        }
    }
    fn branch(start: usize, end: usize, children: Vec<FakeNode>) -> FakeNode {
        FakeNode {
            start,
            end,
            children,
        }
    }
    fn b(start: usize, end: usize) -> ChunkBoundary {
        ChunkBoundary { start, end }
    }

    #[test]
    fn constants_match_semble_defaults() {
        assert_eq!(RECURSION_DEPTH, 500);
        assert_eq!(MIN_CHUNK_SIZE, 50);
    }

    #[test]
    fn recognizes_expanded_language_set() {
        // The original curated set stays supported...
        for lang in [
            "rust",
            "python",
            "javascript",
            "typescript",
            "tsx",
            "go",
            "java",
            "c",
            "cpp",
            "ruby",
            "json",
            "bash",
            "html",
            "css",
        ] {
            assert!(is_supported_language(lang), "{lang} should be supported");
        }
        // ...plus the languages that used to fall through to line chunking and
        // now resolve to a language-pack grammar (full upstream parity). This is
        // a metadata-only check (`has_language`) — it does not download parsers.
        for lang in [
            "kotlin", "swift", "php", "scala", "lua", "csharp", "dart", "elixir", "haskell",
            "ocaml", "zig", "nix", "perl", "r", "julia", "clojure",
        ] {
            assert!(
                is_supported_language(lang),
                "{lang} should be supported by the language pack"
            );
        }
        assert!(!is_supported_language("not-a-real-language"));
    }

    // --- merge_adjacent_chunks ---

    #[test]
    fn merge_empty() {
        assert_eq!(merge_adjacent_chunks(&[], 100), vec![]);
    }

    #[test]
    fn merge_single_passthrough() {
        assert_eq!(merge_adjacent_chunks(&[b(0, 50)], 100), vec![b(0, 50)]);
    }

    #[test]
    fn merge_under_desired() {
        let input = [b(0, 30), b(30, 60), b(60, 80)];
        assert_eq!(merge_adjacent_chunks(&input, 100), vec![b(0, 80)]);
    }

    #[test]
    fn merge_keeps_separate_when_exceeds() {
        let input = [b(0, 60), b(60, 130)];
        assert_eq!(
            merge_adjacent_chunks(&input, 100),
            vec![b(0, 60), b(60, 130)]
        );
    }

    #[test]
    fn merge_greedily_packs() {
        let input = [b(0, 40), b(40, 80), b(80, 130), b(130, 160)];
        assert_eq!(
            merge_adjacent_chunks(&input, 100),
            vec![b(0, 80), b(80, 160)]
        );
    }

    // --- chunk_lines ---

    #[test]
    fn chunk_lines_empty() {
        assert_eq!(chunk_lines("", 100), vec![]);
    }

    #[test]
    fn chunk_lines_whitespace_only() {
        assert_eq!(chunk_lines("   \n\n\t  \n", 100), vec![]);
    }

    #[test]
    fn chunk_lines_short_single_chunk() {
        let src = "hello\nworld\n";
        let chunks = chunk_lines(src, 1500);
        assert_eq!(chunks, vec![b(0, src.chars().count())]);
    }

    #[test]
    fn chunk_lines_contiguous_cover() {
        let src: String = (0..200).map(|i| format!("line {i}\n")).collect();
        let chunks = chunk_lines(&src, 500);
        assert_eq!(chunks[0].start, 0);
        assert_eq!(chunks.last().unwrap().end, src.chars().count());
        for w in chunks.windows(2) {
            assert_eq!(w[1].start, w[0].end);
        }
    }

    #[test]
    fn chunk_lines_preserves_crlf() {
        let src = "a\r\nb\r\nc\r\n";
        assert_eq!(chunk_lines(src, 1500), vec![b(0, src.chars().count())]);
    }

    // --- merge_node / merge_node_inner ---

    #[test]
    fn inner_single_boundary_for_leaf() {
        assert_eq!(merge_node_inner(&leaf(10, 60), 100, 0), vec![b(10, 60)]);
    }

    #[test]
    fn inner_no_recurse_below_min_chunk_size() {
        let root = branch(0, 40, vec![leaf(0, 20), leaf(20, 40)]);
        assert_eq!(merge_node_inner(&root, 100, 0), vec![b(0, 40)]);
    }

    #[test]
    fn inner_caps_recursion_depth() {
        let root = branch(0, 200, vec![leaf(0, 100), leaf(100, 200)]);
        assert_eq!(
            merge_node_inner(&root, 50, RECURSION_DEPTH + 1),
            vec![b(0, 200)]
        );
    }

    #[test]
    fn inner_groups_children_up_to_desired() {
        let root = branch(
            0,
            300,
            vec![leaf(0, 40), leaf(40, 80), leaf(80, 200), leaf(200, 300)],
        );
        assert_eq!(
            merge_node_inner(&root, 100, 0),
            vec![b(0, 80), b(80, 200), b(200, 300)]
        );
    }

    #[test]
    fn merge_node_merges_adjacent_inner_groups() {
        let root = branch(0, 150, vec![leaf(0, 30), leaf(30, 60), leaf(60, 150)]);
        assert_eq!(merge_node(&root, 100), vec![b(0, 60), b(60, 150)]);
    }

    // --- chunk (tree-sitter) ---

    #[test]
    fn chunk_whitespace_returns_empty() {
        assert_eq!(chunk("   \n\t\n", "typescript", 1500), Some(vec![]));
        assert_eq!(chunk("", "python", 1500), Some(vec![]));
    }

    #[test]
    fn chunk_returns_none_without_parser() {
        // An unknown language is rejected by the language pack's bundled manifest
        // without any network access, so `chunk` returns `None` → line fallback.
        assert_eq!(
            chunk("let x = 1\n", "__definitely_not_a_real_language__", 1500),
            None
        );
    }

    // The remaining `chunk(...)` tests parse real source, which makes the language
    // pack download the grammar from GitHub releases on first use (then cache it).
    // They are `#[ignore]`d so the default `cargo test` stays offline; run them
    // with `cargo test -- --ignored` where network is available.

    #[test]
    #[ignore = "downloads a tree-sitter grammar from GitHub releases"]
    fn chunk_parses_real_rust_into_covering_boundaries() {
        let src = "fn a() {\n    let x = 1;\n}\n\nfn b() {\n    let y = 2;\n}\n";
        let boundaries = chunk(src, "rust", 1500).expect("rust is supported → Some");
        assert!(!boundaries.is_empty());
        // Boundaries are character offsets within the source.
        let n = src.chars().count();
        for b in &boundaries {
            assert!(
                b.start <= b.end && b.end <= n,
                "boundary {b:?} out of range"
            );
        }
        // A small desired length splits the two functions into separate chunks.
        let split = chunk(src, "rust", 20).expect("Some");
        assert!(
            split.len() >= 2,
            "small desired_length should split: {split:?}"
        );
    }

    #[test]
    #[ignore = "downloads a tree-sitter grammar from GitHub releases"]
    fn chunk_byte_to_char_handles_multibyte() {
        // A multibyte comment ensures byte→char conversion doesn't over-count.
        let src = "// café ☕ a comment\nfn z() {}\n";
        let boundaries = chunk(src, "rust", 1500).expect("Some");
        let n = src.chars().count();
        for b in &boundaries {
            assert!(b.end <= n, "char boundary {b:?} exceeds char count {n}");
        }
    }

    #[test]
    #[ignore = "downloads tree-sitter grammars from GitHub releases"]
    fn newly_supported_languages_are_ast_chunked() {
        // Languages outside the old curated set now AST-chunk instead of falling
        // back to line chunking. A small `desired_length` forces a split that the
        // AST honors at declaration boundaries.
        let cases = [
            (
                "kotlin",
                "fun a() {\n    val x = 1\n}\n\nfun b() {\n    val y = 2\n}\n",
            ),
            (
                "swift",
                "func a() {\n    let x = 1\n}\n\nfunc b() {\n    let y = 2\n}\n",
            ),
            (
                "php",
                "<?php\nfunction a() {\n    $x = 1;\n}\nfunction b() {\n    $y = 2;\n}\n",
            ),
        ];
        for (lang, src) in cases {
            let boundaries = chunk(src, lang, 1500)
                .unwrap_or_else(|| panic!("{lang} should AST-chunk → Some, not line fallback"));
            assert!(!boundaries.is_empty(), "{lang}: expected boundaries");
            let n = src.chars().count();
            for b in &boundaries {
                assert!(
                    b.start <= b.end && b.end <= n,
                    "{lang}: boundary {b:?} out of range (n={n})"
                );
            }
            let split = chunk(src, lang, 20).expect("Some");
            assert!(
                split.len() >= 2,
                "{lang}: small desired_length should split: {split:?}"
            );
        }
    }
}
