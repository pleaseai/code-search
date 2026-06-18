//! AST-based chunker with a line-based fallback. Port of
//! `src/chunking/core.ts` (← semble `chunking/core.py`).
//!
//! The merge algorithm is generic over [`AstNode`] so it can be unit-tested
//! with mock nodes and later driven by `tree_sitter::Node`. Real tree-sitter
//! parsing activates together with the language map (T012); until then
//! `is_supported_language` returns `false` (matching the upstream `ALL_LANGUAGES`
//! stub) and callers use the line fallback.

pub const RECURSION_DEPTH: usize = 500;
pub const MIN_CHUNK_SIZE: usize = 50;

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

/// Check if the language is supported by tree-sitter. Currently always `false`
/// (the upstream `ALL_LANGUAGES` is an empty stub pending the language map).
pub fn is_supported_language(_language: &str) -> bool {
    false
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
/// input, and `None` when no parser is available for `language` (callers fall
/// back to [`chunk_lines`]).
///
/// Until language grammars are registered (T012), no parser is available, so
/// this returns `None` for any non-whitespace input — matching the upstream
/// lazy-load fallback when the tree-sitter dependency is absent.
pub fn chunk(text: &str, _language: &str, _desired_length: usize) -> Option<Vec<ChunkBoundary>> {
    if text.trim().is_empty() {
        return Some(Vec::new());
    }
    None
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
    fn unsupported_language_stub() {
        assert!(!is_supported_language("typescript"));
        assert!(!is_supported_language("python"));
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
        assert_eq!(
            chunk("let x = 1\n", "__definitely_not_a_real_language__", 1500),
            None
        );
    }
}
