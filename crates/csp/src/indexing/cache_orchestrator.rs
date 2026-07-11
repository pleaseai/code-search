//! Build-or-reuse orchestration for the global on-disk index cache. Port of
//! `src/indexing/cache.ts`.

use std::path::{Path, PathBuf};

use crate::chunking::source::DESIRED_CHUNK_LENGTH_CHARS;
use crate::indexing::cache::{
    compute_content_hash_from_paths, ensure_cache_dir, resolve_cache_dir, CacheLocation,
};
use crate::indexing::create::MAX_FILE_BYTES;
use crate::indexing::dense::DEFAULT_MODEL_NAME;
use crate::indexing::file_walker::walk_files;
use crate::indexing::files::get_extensions;
use crate::indexing::index::{normalize_content, parse_manifest, CspIndex, LoadOptions};
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
    let source_hash = if is_git {
        None
    } else {
        Some(compute_content_hash_from_paths(collect_source_paths(
            Path::new(source),
            &content,
        )))
    };

    // The resolved model name the index would be (re)built with. A cache built
    // with a different model must not be reused — its vectors are incompatible
    // (mirrors semble#219 bumping the default to the `-v2` weights).
    let expected_model = options.model_path.as_deref().unwrap_or(DEFAULT_MODEL_NAME);

    if let Some(cached) = try_reuse(&cache_dir, is_git, source_hash.as_deref(), expected_model) {
        return Ok(cached);
    }

    let load_options = LoadOptions {
        model_path: options.model_path.clone(),
        content: Some(content),
    };
    let index = if is_git {
        CspIndex::from_git(source, &load_options, options.git_ref.as_deref())?
    } else {
        CspIndex::from_path(Path::new(source), &load_options)?
    };
    index.save(&cache_dir, source_hash.as_deref())?;
    Ok(index)
}

/// Reuse a cached index when present and valid, else `None`.
fn try_reuse(
    cache_dir: &Path,
    is_git: bool,
    source_hash: Option<&str>,
    expected_model: &str,
) -> Option<CspIndex> {
    let manifest_path = cache_dir.join("manifest.json");
    if !manifest_path.exists() {
        return None;
    }
    let raw = std::fs::read_to_string(&manifest_path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let manifest = parse_manifest(&value).ok()?;
    // A chunk_size change re-chunks every file, so a cache built with a different
    // target length is stale even if the source files are byte-identical.
    if manifest.chunk_size != Some(DESIRED_CHUNK_LENGTH_CHARS as u32) {
        return None;
    }
    // A model change makes the persisted vectors incompatible with queries
    // embedded by the new model, so a cache built with a different model is stale.
    if manifest.model_id != expected_model {
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
