//! Index orchestration. Port of semble `index/create.py` (incremental reuse
//! from upstream #225).
//!
//! Walks files matching the resolved extensions, chunks them, enriches +
//! tokenizes text for BM25, embeds the chunks, and returns the populated
//! sparse/dense indexes alongside the chunk list and a per-file manifest.
//! When a [`PreviousIndex`] is supplied, files whose content hash is unchanged
//! reuse their previous chunks, vector rows, and BM25 postings; only changed
//! files are re-chunked and re-embedded, and deleted files' postings are
//! dropped.

use std::path::{Path, PathBuf};

use crate::chunking::source::chunk_source;
use crate::indexing::cache::sha256_hex;
use crate::indexing::dense::{embed_chunk_refs, Model, SelectableBasicBackend};
use crate::indexing::file_walker::walk_files;
use crate::indexing::files::{detect_language, get_extensions};
use crate::indexing::sparse::{enrich_for_bm25, Bm25Index};
use crate::indexing::types::{make_chunk_id, FileManifest, FileManifestEntry, PreviousIndex};
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
    /// Per-file content hash + chunk range, for the next incremental reindex.
    pub files: FileManifest,
}

/// Replace a file's BM25 postings: remove its old slots (if any), then add its
/// new ones.
fn reindex_file(
    bm25_index: &mut Bm25Index,
    indexed_path: &str,
    file_chunks: &[Chunk],
    previous_entry: Option<&FileManifestEntry>,
) -> Result<(), String> {
    if let Some(entry) = previous_entry {
        for slot in 0..entry.count {
            bm25_index.remove_document(&make_chunk_id(indexed_path, slot));
        }
    }
    for (slot, chunk) in file_chunks.iter().enumerate() {
        bm25_index.add_document(
            &make_chunk_id(indexed_path, slot),
            &tokenize(&enrich_for_bm25(chunk)),
        )?;
    }
    Ok(())
}

/// Create an index from a resolved directory, optionally reusing a previous
/// index's unchanged files. Errors when no chunks are produced.
pub fn create_index_from_path(
    path: &Path,
    options: &CreateIndexOptions,
    previous: Option<PreviousIndex>,
) -> Result<CreateIndexResult, String> {
    let content = options
        .content
        .clone()
        .unwrap_or_else(|| vec![ContentType::Code]);
    let resolved = get_extensions(&content, options.extensions.as_deref());
    let ext_refs: Vec<&str> = resolved.iter().map(String::as_str).collect();

    // The previous index is consumed: its BM25 index is mutated in place and its
    // chunk/vector rows are moved out rather than copied.
    let (mut bm25_index, previous_files, mut previous_chunks, mut previous_vectors) = match previous
    {
        Some(prev) => (
            prev.bm25_index,
            prev.files,
            prev.chunks.into_iter().map(Some).collect::<Vec<_>>(),
            prev.vectors.into_iter().map(Some).collect::<Vec<_>>(),
        ),
        None => (
            Bm25Index::new(),
            FileManifest::new(),
            Vec::new(),
            Vec::new(),
        ),
    };

    let mut chunks: Vec<Chunk> = Vec::new();
    let mut chunk_ids: Vec<String> = Vec::new();
    // Reused rows land here directly; freshly chunked files leave a `None` hole
    // that the single batched embed pass below fills, so the tokenizer keeps the
    // whole-corpus batching it had before incremental reuse existed.
    let mut vectors: Vec<Option<Vec<f32>>> = Vec::new();
    let mut fresh_rows: Vec<usize> = Vec::new();
    let mut files = FileManifest::new();

    for file_path in walk_files(path, &ext_refs, &[]) {
        let language = detect_language(&file_path.to_string_lossy());
        let size = match std::fs::metadata(&file_path) {
            Ok(meta) => meta.len(),
            Err(_) => continue,
        };
        if size > MAX_FILE_BYTES {
            continue;
        }
        let Ok(bytes) = std::fs::read(&file_path) else {
            continue;
        };
        let hash = sha256_hex(&bytes);
        let indexed_path = match &options.display_root {
            Some(root) => file_path
                .strip_prefix(root)
                .unwrap_or(&file_path)
                .to_string_lossy()
                .into_owned(),
            None => file_path.to_string_lossy().into_owned(),
        };
        let previous_entry = previous_files.get(&indexed_path);

        // Unchanged file: move its previous chunk + vector rows out (each row is
        // taken at most once because a validated manifest's ranges never overlap).
        let reused = match previous_entry {
            Some(entry)
                if entry.hash == hash
                    && entry.end() <= previous_chunks.len()
                    && entry.end() <= previous_vectors.len() =>
            {
                let rows: Option<Vec<Chunk>> = previous_chunks[entry.start..entry.end()]
                    .iter_mut()
                    .map(Option::take)
                    .collect();
                let vecs: Option<Vec<Vec<f32>>> = previous_vectors[entry.start..entry.end()]
                    .iter_mut()
                    .map(Option::take)
                    .collect();
                rows.zip(vecs)
            }
            _ => None,
        };
        let start = chunks.len();
        let file_chunks = match reused {
            Some((file_chunks, file_vectors)) => {
                vectors.extend(file_vectors.into_iter().map(Some));
                file_chunks
            }
            None => {
                // Lossy UTF-8 decode (invalid bytes → U+FFFD): only an IO error
                // skips a file, never an encoding error.
                let source = String::from_utf8_lossy(&bytes).into_owned();
                let file_chunks = chunk_source(&source, &indexed_path, language);
                reindex_file(&mut bm25_index, &indexed_path, &file_chunks, previous_entry)?;
                fresh_rows.extend(start..start + file_chunks.len());
                vectors.extend(std::iter::repeat_n(None, file_chunks.len()));
                file_chunks
            }
        };

        let count = file_chunks.len();
        chunk_ids.extend((0..count).map(|slot| make_chunk_id(&indexed_path, slot)));
        chunks.extend(file_chunks);
        files.insert(indexed_path, FileManifestEntry { hash, start, count });
    }

    // Files that vanished since the previous index: drop their postings.
    for (indexed_path, entry) in &previous_files {
        if !files.contains_key(indexed_path) {
            reindex_file(&mut bm25_index, indexed_path, &[], Some(entry))?;
        }
    }

    if chunks.is_empty() {
        return Err(format!(
            "No supported files found under {}.",
            path.display()
        ));
    }

    // One batched embed for every changed file's chunks — the tokenizer
    // parallelises per batch, so a call per file would serialise a cold build.
    // Normalise through the backend so fresh rows match the reused
    // (already-normalised) rows.
    let fresh_chunks: Vec<&Chunk> = fresh_rows.iter().map(|&i| &chunks[i]).collect();
    let fresh_vectors =
        SelectableBasicBackend::from_vectors(embed_chunk_refs(options.model, &fresh_chunks))?
            .vectors;
    if fresh_vectors.len() != fresh_rows.len() {
        return Err("Embedder returned the wrong number of rows".to_string());
    }
    for (&row, vector) in fresh_rows.iter().zip(fresh_vectors) {
        vectors[row] = Some(vector);
    }
    let vectors: Option<Vec<Vec<f32>>> = vectors.into_iter().collect();
    let vectors = vectors.ok_or("Internal error: an embedding row was left unfilled")?;

    bm25_index.set_doc_order(chunk_ids);
    let semantic_index = SelectableBasicBackend::from_normalized(vectors)?;

    Ok(CreateIndexResult {
        bm25_index,
        semantic_index,
        chunks,
        files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexing::dense::make_stub_model;
    use crate::tokens::tokenize;
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
        let result = create_index_from_path(
            dir.path(),
            &opts(&model, Some(dir.path().to_path_buf())),
            None,
        )
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
        let err = create_index_from_path(dir.path(), &opts(&model, None), None).unwrap_err();
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
        let result = create_index_from_path(dir.path(), &options, None).unwrap();
        assert_eq!(result.chunks.len(), 1);
        assert_eq!(result.chunks[0].file_path, "a.txt");
    }

    #[test]
    fn skips_files_over_max_bytes() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("big.ts"), "a".repeat(2_000_000)).unwrap();
        std::fs::write(dir.path().join("small.ts"), "export const x = 1\n").unwrap();
        let model = make_stub_model(4);
        let result = create_index_from_path(
            dir.path(),
            &opts(&model, Some(dir.path().to_path_buf())),
            None,
        )
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
        let result = create_index_from_path(
            dir.path(),
            &opts(&model, Some(dir.path().to_path_buf())),
            None,
        )
        .unwrap();
        assert!(result
            .chunks
            .iter()
            .any(|c| c.file_path.ends_with("nested.ts")));
    }

    // --- incremental reindex (mirrors upstream tests/index/test_create.py) ---

    fn write_files(root: &Path, files: &[(&str, &str)]) {
        for (rel, content) in files {
            let path = root.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, content).unwrap();
        }
    }

    fn into_previous(result: CreateIndexResult) -> PreviousIndex {
        PreviousIndex::try_new(
            result.chunks,
            result.semantic_index.vectors,
            result.files,
            result.bm25_index,
        )
        .unwrap()
    }

    fn score_sum(index: &Bm25Index, query: &str) -> f32 {
        index.get_scores(&tokenize(query), None).iter().sum()
    }

    #[test]
    fn fresh_build_records_a_layout_valid_file_manifest() {
        let dir = tempdir().unwrap();
        write_files(
            dir.path(),
            &[
                ("a.ts", "function stable_anchor() { return 1 }\n"),
                ("sub/b.ts", "function other_value() { return 2 }\n"),
            ],
        );
        let model = make_stub_model(4);
        let result = create_index_from_path(
            dir.path(),
            &opts(&model, Some(dir.path().to_path_buf())),
            None,
        )
        .unwrap();

        assert_eq!(result.files.len(), 2);
        let total: usize = result.files.values().map(|e| e.count).sum();
        assert_eq!(total, result.chunks.len());
        for (path, entry) in &result.files {
            assert_eq!(entry.hash.len(), 64);
            assert!(result.chunks[entry.start..entry.end()]
                .iter()
                .all(|c| c.file_path == *path));
        }
        // The manifest, chunks, vectors, and BM25 order all agree.
        assert!(into_previous(result).files.contains_key("a.ts"));
    }

    #[test]
    fn incremental_reindex_reuses_updates_and_prunes() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();
        write_files(
            &root,
            &[
                ("a.ts", "function stable_anchor() { return 1 }\n"),
                ("b.ts", "function changed_value() { return 2 }\n"),
                ("c.ts", "function unique_gone() { return 3 }\n"),
                ("emptying.ts", "function becomes_empty() { return 4 }\n"),
            ],
        );
        let model = make_stub_model(4);
        let before =
            create_index_from_path(&root, &opts(&model, Some(root.clone())), None).unwrap();
        let b_before = before.files["b.ts"].clone();
        let b_vectors_before =
            before.semantic_index.vectors[b_before.start..b_before.end()].to_vec();
        let a_before = before.files["a.ts"].clone();

        // Plant sentinels in the previous index for the unchanged file: they can
        // only survive into the rebuilt index if its rows were reused, not
        // re-chunked/re-embedded (the stub embedder is deterministic).
        let mut previous = into_previous(before);
        let sentinel_vector = vec![0.0, 1.0, 0.0, 0.0];
        previous.vectors[a_before.start] = sentinel_vector.clone();
        previous.chunks[a_before.start]
            .content
            .push_str("/*reused*/");

        write_files(
            &root,
            &[("b.ts", "function changed_value() { return 999 }\n")],
        );
        std::fs::remove_file(root.join("c.ts")).unwrap();
        write_files(&root, &[("emptying.ts", &" ".repeat(128))]);
        write_files(
            &root,
            &[("d.ts", "function brand_new_term() { return 4 }\n")],
        );

        let after =
            create_index_from_path(&root, &opts(&model, Some(root.clone())), Some(previous))
                .unwrap();

        let a_after = &after.files["a.ts"];
        assert_eq!(after.semantic_index.vectors[a_after.start], sentinel_vector);
        assert!(after.chunks[a_after.start].content.ends_with("/*reused*/"));
        let b_after = &after.files["b.ts"];
        assert_ne!(
            after.semantic_index.vectors[b_after.start..b_after.end()].to_vec(),
            b_vectors_before
        );
        assert!(!after.files.contains_key("c.ts"));
        assert!(after.files.contains_key("d.ts"));
        assert_eq!(after.files["emptying.ts"].count, 0);

        assert_eq!(score_sum(&after.bm25_index, "unique_gone"), 0.0);
        assert_eq!(score_sum(&after.bm25_index, "becomes_empty"), 0.0);
        assert!(score_sum(&after.bm25_index, "brand_new_term") > 0.0);
        assert!(score_sum(&after.bm25_index, "changed_value") > 0.0);

        let mut expected_ids: Vec<String> = after
            .files
            .iter()
            .flat_map(|(path, entry)| (0..entry.count).map(move |slot| make_chunk_id(path, slot)))
            .collect();
        expected_ids.sort();
        let mut doc_order = after.bm25_index.doc_order().to_vec();
        doc_order.sort();
        assert_eq!(doc_order, expected_ids);
        assert_eq!(after.bm25_index.corpus_size(), after.chunks.len());
        assert_eq!(after.semantic_index.vectors.len(), after.chunks.len());
        // The rebuilt index is itself a valid seed for the next incremental pass.
        into_previous(after);
    }

    #[test]
    fn zero_chunk_file_does_not_break_manifest_tiling() {
        let dir = tempdir().unwrap();
        let root = dir.path().to_path_buf();
        // `pkg/` is walked before `pkg.ts` (directory entries sort by file name),
        // but "pkg.ts" < "pkg/z.ts" lexicographically ('.' 0x2E < '/' 0x2F). The
        // empty file yields no chunks, so both entries share `start`.
        write_files(
            &root,
            &[
                ("pkg/z.ts", "   \n"),
                ("pkg.ts", "function stable_anchor() { return 1 }\n"),
            ],
        );
        let model = make_stub_model(4);
        let result =
            create_index_from_path(&root, &opts(&model, Some(root.clone())), None).unwrap();
        assert_eq!(result.files["pkg/z.ts"].count, 0);
        assert_eq!(result.files["pkg/z.ts"].start, result.files["pkg.ts"].start);
        // A freshly built index must always be a valid seed for the next pass.
        into_previous(result);
    }
}
