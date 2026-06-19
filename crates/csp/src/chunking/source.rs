//! Public chunking entry point. Port of `src/chunking/chunk-source.ts`
//! (← semble `chunking/chunking.py`).
//!
//! Takes raw source text and a language hint and returns concrete [`Chunk`]
//! values with 1-indexed line numbers, using the AST chunker when the language
//! is supported and the line fallback otherwise. Only `\n` counts as a newline
//! for line numbering (semble parity).

use super::core::{chunk as chunk_ast, chunk_lines, is_supported_language, ChunkBoundary};
use crate::types::Chunk;

/// The desired length of chunks in characters.
pub const DESIRED_CHUNK_LENGTH_CHARS: usize = 1500;

/// Chunk pre-read source text.
pub fn chunk_source(source: &str, file_path: &str, language: Option<&str>) -> Vec<Chunk> {
    if source.trim().is_empty() {
        return Vec::new();
    }

    let mut boundaries: Option<Vec<ChunkBoundary>> = None;
    if let Some(lang) = language {
        if is_supported_language(lang) {
            boundaries = chunk_ast(source, lang, DESIRED_CHUNK_LENGTH_CHARS);
        }
    }
    // `if` (not `else`): the parser's error state is `None` — fall through to
    // the line chunker.
    let boundaries = boundaries.unwrap_or_else(|| chunk_lines(source, DESIRED_CHUNK_LENGTH_CHARS));

    // Resolve 1-indexed line numbers in a single forward pass over the source's
    // characters (boundaries are sorted by start offset).
    let chars: Vec<char> = source.chars().collect();
    let mut chunks = Vec::new();
    let mut cursor = 0usize;
    let mut line = 1u32;

    for boundary in boundaries {
        // Clamp to start so zero-length chunks don't produce an off-by-one.
        let end_index = boundary.end.saturating_sub(1).max(boundary.start);
        let upper = (end_index + 1).min(chars.len());
        let content: String = chars[boundary.start.min(chars.len())..upper]
            .iter()
            .collect();
        line = advance_to(&chars, &mut cursor, boundary.start, line);
        let start_line = line;
        line = advance_to(&chars, &mut cursor, end_index, line);
        let end_line = line;
        chunks.push(Chunk {
            content,
            file_path: file_path.to_string(),
            start_line,
            end_line,
            language: language.map(str::to_string),
        });
    }

    chunks
}

/// Advance `cursor` to `target` (clamped to the source length), counting `\n`
/// newlines, and return the resulting 1-indexed line.
fn advance_to(chars: &[char], cursor: &mut usize, target: usize, mut line: u32) -> u32 {
    let limit = target.min(chars.len());
    while *cursor < limit {
        if chars[*cursor] == '\n' {
            line += 1;
        }
        *cursor += 1;
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_source() {
        assert_eq!(chunk_source("", "foo.txt", None), vec![]);
    }

    #[test]
    fn whitespace_only_source() {
        assert_eq!(chunk_source("   \n\t\n  ", "foo.txt", None), vec![]);
    }

    #[test]
    fn short_plain_text_single_chunk() {
        let chunks = chunk_source("hello\nworld\n", "foo.txt", None);
        assert_eq!(chunks.len(), 1);
        let c = &chunks[0];
        assert_eq!(c.file_path, "foo.txt");
        assert_eq!(c.language, None);
        assert_eq!(c.start_line, 1);
        assert_eq!(c.end_line, 2);
        assert!(c.content.starts_with("hello\nworld"));
    }

    #[test]
    fn long_source_chunks_within_desired_length() {
        let line = format!("{}\n", "x".repeat(49)); // 50 chars/line
        let src = line.repeat(60); // 3000 chars
        assert_eq!(src.chars().count(), 3000);
        let chunks = chunk_source(&src, "big.txt", None);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert!(c.content.chars().count() <= DESIRED_CHUNK_LENGTH_CHARS);
        }
    }

    #[test]
    fn one_indexed_line_numbers() {
        let chunks = chunk_source("line1\nline2\nline3\nline4\n", "foo.txt", None);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].end_line, 4);
    }

    #[test]
    fn falls_back_for_unsupported_language() {
        let chunks = chunk_source("a\nb\nc\n", "foo.xyz", Some("not-a-real-language"));
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].start_line, 1);
        assert_eq!(chunks[0].language.as_deref(), Some("not-a-real-language"));
    }

    #[test]
    fn preserves_file_path_on_every_chunk() {
        let src = format!("{}\n", "a".repeat(100)).repeat(50);
        let chunks = chunk_source(&src, "path/to/file.txt", None);
        assert!(!chunks.is_empty());
        for c in &chunks {
            assert_eq!(c.file_path, "path/to/file.txt");
        }
    }

    #[test]
    fn multi_chunk_line_ranges_are_contiguous() {
        let lines: Vec<String> = (0..100)
            .map(|i| format!("{i:03} {}", "x".repeat(35)))
            .collect();
        let src = format!("{}\n", lines.join("\n"));
        let chunks = chunk_source(&src, "foo.txt", None);
        assert!(chunks.len() >= 2);
        assert_eq!(chunks[0].start_line, 1);
        for w in chunks.windows(2) {
            assert!(w[1].start_line >= w[0].end_line);
        }
    }
}
