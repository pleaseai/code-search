//! Index orchestration. Port of `src/indexing/create.ts`
//! (← semble `index/create.py`).
//!
//! Walks files matching the resolved extensions, chunks them, enriches +
//! tokenizes text for BM25, embeds the chunks, and returns the populated
//! sparse/dense indexes alongside the chunk list.

use std::path::{Path, PathBuf};

use crate::chunking::source::chunk_source;
use crate::indexing::dense::{embed_chunks, Model, SelectableBasicBackend};
use crate::indexing::file_walker::walk_files;
use crate::indexing::files::{detect_language, get_extensions};
use crate::indexing::sparse::{enrich_for_bm25, Bm25Index};
use crate::tokens::tokenize;
use crate::types::{Chunk, ContentType};

/// 1 MB max file size to read and index.
pub const MAX_FILE_BYTES: u64 = 1_000_000;

/// Options for [`create_index_from_path`].
pub struct CreateIndexOptions<'a> {
    pub model: &'a Model,
    /// Extra extensions appended to those resolved from `content`.
    pub extensions: Option<Vec<String>>,
    /// Content selection (defaults to code-only, matching semble `_DEFAULT_CONTENT`).
    pub content: Option<Vec<ContentType>>,
    /// When set, chunk file paths are stored relative to this root.
    pub display_root: Option<PathBuf>,
}

/// Result of [`create_index_from_path`].
#[derive(Debug)]
pub struct CreateIndexResult {
    pub bm25_index: Bm25Index,
    pub semantic_index: SelectableBasicBackend,
    pub chunks: Vec<Chunk>,
}

/// Create an index from a resolved directory. Errors when no chunks are produced.
pub fn create_index_from_path(
    path: &Path,
    options: &CreateIndexOptions,
) -> Result<CreateIndexResult, String> {
    let content = options
        .content
        .clone()
        .unwrap_or_else(|| vec![ContentType::Code]);
    let resolved = get_extensions(&content, options.extensions.as_deref());
    let ext_refs: Vec<&str> = resolved.iter().map(String::as_str).collect();

    let mut chunks: Vec<Chunk> = Vec::new();
    for file_path in walk_files(path, &ext_refs, &[]) {
        let language = detect_language(&file_path.to_string_lossy());
        let size = match std::fs::metadata(&file_path) {
            Ok(meta) => meta.len(),
            Err(_) => continue,
        };
        if size > MAX_FILE_BYTES {
            continue;
        }
        // Lossy UTF-8 decode (invalid bytes → U+FFFD) to match the TS oracle's
        // `readFileSync(path, 'utf8')`, which decodes lossily and only skips on
        // an IO error — `read_to_string` would instead drop the whole file.
        let source = match std::fs::read(&file_path) {
            Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
            Err(_) => continue,
        };
        let chunk_path = match &options.display_root {
            Some(root) => file_path
                .strip_prefix(root)
                .unwrap_or(&file_path)
                .to_string_lossy()
                .into_owned(),
            None => file_path.to_string_lossy().into_owned(),
        };
        chunks.extend(chunk_source(&source, &chunk_path, language));
    }

    if chunks.is_empty() {
        return Err(format!(
            "No supported files found under {}.",
            path.display()
        ));
    }

    let embeddings = embed_chunks(options.model, &chunks);
    let documents: Vec<Vec<String>> = chunks
        .iter()
        .map(|c| tokenize(&enrich_for_bm25(c)))
        .collect();
    let bm25_index = Bm25Index::build(&documents);
    let semantic_index = SelectableBasicBackend::from_vectors(embeddings)?;

    Ok(CreateIndexResult {
        bm25_index,
        semantic_index,
        chunks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexing::dense::make_stub_model;
    use tempfile::tempdir;

    fn opts(model: &Model, display_root: Option<PathBuf>) -> CreateIndexOptions<'_> {
        CreateIndexOptions {
            model,
            extensions: None,
            content: None,
            display_root,
        }
    }

    #[test]
    fn builds_indexes_for_small_ts_file() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("sample.ts"),
            "export function greet(name: string) {\n  return `hi ${name}`\n}\n",
        )
        .unwrap();
        let model = make_stub_model(4);
        let result =
            create_index_from_path(dir.path(), &opts(&model, Some(dir.path().to_path_buf())))
                .unwrap();

        assert!(!result.chunks.is_empty());
        assert_eq!(result.chunks[0].file_path, "sample.ts");
        assert_eq!(result.semantic_index.vectors.len(), result.chunks.len());
        assert_eq!(result.bm25_index.num_docs(), result.chunks.len());
    }

    #[test]
    fn errors_when_no_supported_files() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("data.bin"), "binary").unwrap();
        let model = make_stub_model(4);
        let err = create_index_from_path(dir.path(), &opts(&model, None)).unwrap_err();
        assert!(err.contains("No supported files found"));
    }

    #[test]
    fn respects_extensions_override() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "hello world").unwrap();
        let model = make_stub_model(4);
        let options = CreateIndexOptions {
            model: &model,
            extensions: Some(vec![".txt".to_string()]),
            content: Some(vec![ContentType::Docs]),
            display_root: Some(dir.path().to_path_buf()),
        };
        let result = create_index_from_path(dir.path(), &options).unwrap();
        assert_eq!(result.chunks.len(), 1);
        assert_eq!(result.chunks[0].file_path, "a.txt");
    }

    #[test]
    fn skips_files_over_max_bytes() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("big.ts"), "a".repeat(2_000_000)).unwrap();
        std::fs::write(dir.path().join("small.ts"), "export const x = 1\n").unwrap();
        let model = make_stub_model(4);
        let result =
            create_index_from_path(dir.path(), &opts(&model, Some(dir.path().to_path_buf())))
                .unwrap();
        let paths: Vec<&str> = result.chunks.iter().map(|c| c.file_path.as_str()).collect();
        assert!(paths.contains(&"small.ts"));
        assert!(!paths.contains(&"big.ts"));
    }

    #[test]
    fn descends_into_subdirectories() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/nested.ts"), "const a = 1\n").unwrap();
        let model = make_stub_model(4);
        let result =
            create_index_from_path(dir.path(), &opts(&model, Some(dir.path().to_path_buf())))
                .unwrap();
        assert!(result
            .chunks
            .iter()
            .any(|c| c.file_path.ends_with("nested.ts")));
    }
}
