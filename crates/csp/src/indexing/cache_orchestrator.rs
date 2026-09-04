//! Build-or-reuse orchestration for the global on-disk index cache (the
//! `get_validated_cache` / `load_previous_for_incremental` half of semble
//! `cache.py`, adapted to csp's content-hash oracle — ADR-0002 / ADR-0005).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::chunking::source::DESIRED_CHUNK_LENGTH_CHARS;
use crate::indexing::cache::{
    compute_content_hash_from_paths, ensure_cache_dir, resolve_cache_dir, CacheLocation,
};
use crate::indexing::create::MAX_FILE_BYTES;
use crate::indexing::dense::SelectableBasicBackend;
use crate::indexing::dense::DEFAULT_MODEL_NAME;
use crate::indexing::file_walker::walk_files;
use crate::indexing::files::get_extensions;
use crate::indexing::index::{
    normalize_content, parse_manifest, read_chunks, CspIndex, IndexManifest, LoadOptions,
    INDEX_SCHEMA_VERSION,
};
use crate::indexing::sparse::Bm25Index;
use crate::indexing::types::PreviousIndex;
use crate::types::ContentType;
use crate::utils::is_git_url;

/// Options for [`load_or_build_index`].
#[derive(Debug, Clone, Default)]
pub struct LoadOrBuildOptions {
    pub base_dir: Option<PathBuf>,
    pub git_ref: Option<String>,
    pub content: Option<Vec<ContentType>>,
    pub model_path: Option<String>,
}

fn normalized_hash_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Collect the source files `from_path` would index as sorted hash inputs.
/// File contents are read later, one at a time, by the hashing helper.
fn collect_source_paths(root: &Path, content: &[ContentType]) -> Vec<(String, PathBuf)> {
    let resolved = get_extensions(content, None);
    let ext_refs: Vec<&str> = resolved.iter().map(String::as_str).collect();
    let mut files = Vec::new();
    for file_path in walk_files(root, &ext_refs, &[]) {
        let Ok(meta) = std::fs::metadata(&file_path) else {
            continue;
        };
        if meta.len() > MAX_FILE_BYTES {
            continue;
        }
        let rel = file_path.strip_prefix(root).unwrap_or(&file_path);
        files.push((normalized_hash_path(rel), file_path));
    }
    files
}

/// Content-hash fingerprint of a local source tree — the same oracle
/// [`load_or_build_index`] uses to decide cache validity. Returns `None` for git
/// URLs, which are URL+ref keyed and have no cheap live hash.
pub fn source_fingerprint(source: &str, content: &[ContentType]) -> Option<String> {
    if is_git_url(source) {
        return None;
    }
    Some(compute_content_hash_from_paths(collect_source_paths(
        Path::new(source),
        content,
    )))
}

/// Load a cached index for `source` if fresh, else build, persist, and return.
pub fn load_or_build_index(source: &str, options: &LoadOrBuildOptions) -> Result<CspIndex, String> {
    let content = normalize_content(options.content.clone());
    let is_git = is_git_url(source);

    let location = CacheLocation {
        base_dir: options.base_dir.clone(),
        git_ref: options.git_ref.clone(),
    };
    let cache_dir = resolve_cache_dir(source, &content, &location);
    let base_only = CacheLocation {
        base_dir: options.base_dir.clone(),
        git_ref: None,
    };
    ensure_cache_dir(&cache_dir, &base_only)?;

    // Local sources: the source-file hash is the cache-validity oracle. Git
    // sources are URL+ref keyed (no cheap live hash).
    let source_hash = source_fingerprint(source, &content);

    // The resolved model name the index would be (re)built with. A cache built
    // with a different model must not be reused — its vectors are incompatible
    // (mirrors semble#219 bumping the default to the `-v2` weights).
    let expected_model = options.model_path.as_deref().unwrap_or(DEFAULT_MODEL_NAME);

    if let Some(cached) = try_reuse(&cache_dir, is_git, source_hash.as_deref(), expected_model) {
        return Ok(cached);
    }

    let load_options = LoadOptions {
        model_path: options.model_path.clone(),
        content: Some(content.clone()),
    };
    let index = if is_git {
        CspIndex::from_git(source, &load_options, options.git_ref.as_deref())?
    } else {
        // Stale (or absent) cache: seed the rebuild with the previous index so
        // unchanged files keep their chunks, vectors, and BM25 postings.
        let previous = load_previous_for_incremental(&cache_dir, expected_model, &content);
        CspIndex::from_path_with_previous(Path::new(source), &load_options, previous)?
    };
    index.save(&cache_dir, source_hash.as_deref())?;
    Ok(index)
}

/// Read and parse `<cache_dir>/manifest.json`, or `None` when absent/malformed.
fn read_manifest(cache_dir: &Path) -> Option<IndexManifest> {
    let raw = std::fs::read_to_string(cache_dir.join("manifest.json")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    parse_manifest(&value).ok()
}

/// Whether a persisted manifest describes an index the current build
/// parameters could reuse (mirrors upstream `_metadata_matches`).
fn manifest_compatible(manifest: &IndexManifest, expected_model: &str) -> bool {
    // A schema change means the on-disk artifacts may be laid out differently.
    if manifest.schema_version != INDEX_SCHEMA_VERSION {
        return false;
    }
    // A chunk_size change re-chunks every file, so a cache built with a different
    // target length is stale even if the source files are byte-identical.
    if manifest.chunk_size != Some(DESIRED_CHUNK_LENGTH_CHARS as u32) {
        return false;
    }
    // A model change makes the persisted vectors incompatible with queries
    // embedded by the new model, so a cache built with a different model is stale.
    if manifest.model_id != expected_model {
        return false;
    }
    // The persisted vectors and live query model must use the same runtime
    // implementation. This distinguishes a real Model2Vec cache from an offline
    // deterministic-stub cache even when both share the same requested model id.
    let (query_model, _) = crate::indexing::dense::load_model(Some(expected_model));
    manifest.model_kind.as_deref() == Some(query_model.kind())
}

/// Load a compatible cached index as a seed for incremental reindexing, or
/// `None` when the cache is absent, incompatible, or structurally inconsistent
/// (fails closed — a full rebuild is always correct). Port of upstream
/// `load_previous_for_incremental`.
pub(crate) fn load_previous_for_incremental(
    cache_dir: &Path,
    expected_model: &str,
    content: &[ContentType],
) -> Option<PreviousIndex> {
    let manifest = read_manifest(cache_dir)?;
    if !manifest_compatible(&manifest, expected_model) {
        return None;
    }
    // Compare as sets: a length check plus `contains` would accept
    // `[Code, Code]` against `[Code, Docs]`.
    let manifest_content: BTreeSet<&str> = manifest.content.iter().map(|c| c.as_str()).collect();
    let expected_content: BTreeSet<&str> = content.iter().map(|c| c.as_str()).collect();
    if manifest_content != expected_content || manifest.files.is_empty() {
        return None;
    }

    let chunks = read_chunks(cache_dir).ok()?;
    let backend = SelectableBasicBackend::load(cache_dir).ok()?;
    // The persisted rows are concatenated with freshly embedded ones, so a model
    // whose dimension changed under an unchanged id would make the merged matrix
    // ragged — and `from_normalized` would then hard-error out of the rebuild
    // instead of falling back to it. Fail closed here instead. `load` builds
    // every row with exactly `dim` elements, so the backend's dim covers them all.
    let (query_model, _) = crate::indexing::dense::load_model(Some(expected_model));
    if !backend.vectors.is_empty() && backend.dim != query_model.dim() {
        return None;
    }
    let vectors = backend.vectors;
    let bm25_index = Bm25Index::load(cache_dir).ok()?;
    PreviousIndex::try_new(chunks, vectors, manifest.files, bm25_index).ok()
}

/// Reuse a cached index when present and valid, else `None`.
fn try_reuse(
    cache_dir: &Path,
    is_git: bool,
    source_hash: Option<&str>,
    expected_model: &str,
) -> Option<CspIndex> {
    let manifest = read_manifest(cache_dir)?;
    if !manifest_compatible(&manifest, expected_model) {
        return None;
    }
    // Local sources additionally validate the live source-file hash; git sources
    // are URL+ref keyed (no cheap live hash).
    if !is_git && Some(manifest.content_hash.as_str()) != source_hash {
        return None;
    }
    CspIndex::load_from_disk(cache_dir).ok()
}

#[cfg(test)]
mod tests;
