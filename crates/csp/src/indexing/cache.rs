//! Global on-disk index cache location + content hashing. Port of the *pure*
//! pieces of `src/indexing/cache.ts` (T015):
//!
//! - `resolve_cache_dir` — deterministic cache dir for a (source, content, ref) triple.
//! - `resolve_index_root` — `<home>/index`, parent of every cache leaf.
//! - `compute_content_hash` — order-independent sha256 of a file set.
//! - `ensure_cache_dir` — create the `~/.csp → index → leaf` chain with 0700 permissions (NFR-003), tightening any pre-existing directory (Unix).
//! - `clear_index_cache` — safety-guarded removal of the index root only.
//!
//! The `load_or_build_index` orchestration lands in T016 (it composes `CspIndex`,
//! which depends on the dense index — T013).
//!
//! The cache key JSON (`{"sourceId":…,"content":[…],"ref":…}`) and the
//! content-hash byte stream (`"<utf16-len>:<path>"` + raw bytes) match the TS
//! serialization, so digests agree across implementations.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::types::ContentType;

/// Owner-only permissions for every cache directory (NFR-003).
#[cfg(unix)]
const CACHE_DIR_MODE: u32 = 0o700;

/// Hex characters kept from the full sha256 digest for the cache key.
const KEY_LENGTH: usize = 32;

/// Location overrides shared by the cache helpers.
#[derive(Debug, Default, Clone)]
pub struct CacheLocation {
    /// Override for the `~/.csp` home (defaults to `$HOME/.csp`).
    pub base_dir: Option<PathBuf>,
    /// Git ref participating in the cache key, for `from_git`.
    pub git_ref: Option<String>,
}

/// A single file's identity for content hashing: relative path + raw bytes.
pub struct CacheFile {
    pub path: String,
    pub content: Vec<u8>,
}

/// Outcome of [`clear_index_cache`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearIndexResult {
    /// The index root that was targeted (`<home>/index`).
    pub path: PathBuf,
    /// True when an existing index root was removed.
    pub cleared: bool,
    /// Number of top-level cache entries removed (0 when nothing existed).
    pub entries: usize,
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn cache_home(loc: &CacheLocation) -> PathBuf {
    loc.base_dir
        .clone()
        .unwrap_or_else(|| home_dir().join(".csp"))
}

fn to_hex(digest: &[u8]) -> String {
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn is_url_scheme(source: &str) -> bool {
    let Some(pos) = source.find("://") else {
        return false;
    };
    let scheme = &source[..pos];
    let mut chars = scheme.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() => scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')),
        _ => false,
    }
}

/// POSIX `path.normalize`: collapse `.`/`..`/duplicate slashes, preserving a
/// leading and (non-root) trailing slash.
fn normalize_posix(path: &str) -> String {
    let is_abs = path.starts_with('/');
    let has_trailing = path.len() > 1 && path.ends_with('/');
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => continue,
            ".." => {
                if let Some(&last) = out.last() {
                    if last == ".." {
                        out.push("..");
                    } else {
                        out.pop();
                    }
                } else if !is_abs {
                    out.push("..");
                }
            }
            other => out.push(other),
        }
    }
    let mut joined = out.join("/");
    if is_abs {
        joined.insert(0, '/');
    } else if joined.is_empty() {
        joined.push('.');
    }
    if has_trailing && !joined.ends_with('/') {
        joined.push('/');
    }
    joined
}

/// Normalize a source identity: local paths are path-normalized, URLs (scheme://
/// or scp-style `git@`) kept verbatim.
fn normalize_source(source: &str) -> String {
    if is_url_scheme(source) || source.starts_with("git@") {
        source.to_string()
    } else {
        normalize_posix(source)
    }
}

#[derive(Serialize)]
struct CacheKeyPayload {
    #[serde(rename = "sourceId")]
    source_id: String,
    content: Vec<&'static str>,
    #[serde(rename = "ref")]
    git_ref: Option<String>,
}

/// Resolve the cache directory for an indexed source: `<home>/index/<key>`,
/// where `key` is a sha256 (first 32 hex chars) over the normalized source, the
/// order-normalized content selection, and the optional git ref.
pub fn resolve_cache_dir(source: &str, content: &[ContentType], loc: &CacheLocation) -> PathBuf {
    let mut content_key: Vec<&'static str> = content.iter().map(|c| c.as_str()).collect();
    content_key.sort_unstable();

    let payload = CacheKeyPayload {
        source_id: normalize_source(source),
        content: content_key,
        git_ref: loc.git_ref.clone(),
    };
    let json = serde_json::to_string(&payload).expect("cache key payload is serializable");

    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    let digest = to_hex(&hasher.finalize());

    cache_home(loc).join("index").join(&digest[..KEY_LENGTH])
}

/// The root holding every cached index (`<home>/index`) — the only directory
/// [`clear_index_cache`] may remove.
pub fn resolve_index_root(loc: &CacheLocation) -> PathBuf {
    cache_home(loc).join("index")
}

/// Order-independent sha256 (hex) of a file set: files are sorted by path, then
/// each `"<utf16-len>:<path>"` prefix and the raw content bytes are folded in.
pub fn compute_content_hash(files: &[CacheFile]) -> String {
    let mut sorted: Vec<&CacheFile> = files.iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));

    let mut hasher = Sha256::new();
    for file in sorted {
        let len16 = file.path.encode_utf16().count();
        hasher.update(format!("{len16}:{}", file.path).as_bytes());
        hasher.update(&file.content);
    }
    to_hex(&hasher.finalize())
}

/// Directories from `home` down to `leaf` (inclusive), home-first. When `leaf`
/// is not under `home`, only `leaf` is returned.
fn chain_to(leaf: &Path, home: &Path) -> Vec<PathBuf> {
    let mut segments = Vec::new();
    let mut current = leaf.to_path_buf();
    loop {
        segments.push(current.clone());
        if current == home {
            break;
        }
        let Some(parent) = current.parent() else {
            break;
        };
        if parent == current || !current.starts_with(home) {
            break;
        }
        current = parent.to_path_buf();
    }
    segments.reverse();
    segments
}

/// Ensure the `~/.csp → index → leaf` chain exists with 0700 permissions
/// (Unix), tightening any pre-existing directory in the chain.
pub fn ensure_cache_dir(dir: &Path, loc: &CacheLocation) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|e| format!("failed to create cache dir {}: {e}", dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let home = cache_home(loc);
        for segment in chain_to(dir, &home) {
            std::fs::set_permissions(&segment, std::fs::Permissions::from_mode(CACHE_DIR_MODE))
                .map_err(|e| {
                    format!("failed to set 0700 on cache dir {}: {e}", segment.display())
                })?;
        }
    }
    #[cfg(not(unix))]
    let _ = loc;
    Ok(())
}

/// Remove the cached-index root (`<home>/index`) and report how many entries it
/// held. Safety-critical (AC-015): deletes *only* the `index` directory — the
/// resolved target must be the direct `index` child of the resolved home, so a
/// symlinked or misconfigured root cannot escalate into a wider delete.
pub fn clear_index_cache(loc: &CacheLocation) -> Result<ClearIndexResult, String> {
    let home = cache_home(loc);
    let index_root = resolve_index_root(loc);

    if !index_root.exists() {
        return Ok(ClearIndexResult {
            path: index_root,
            cleared: false,
            entries: 0,
        });
    }

    // Resolve symlinks before the guard so a symlinked `index` (or home) cannot
    // redirect the delete outside the cache tree.
    let real_index_root = std::fs::canonicalize(&index_root).map_err(|e| e.to_string())?;
    let real_home = if home.exists() {
        std::fs::canonicalize(&home).map_err(|e| e.to_string())?
    } else {
        home.clone()
    };

    let basename_ok = real_index_root.file_name().is_some_and(|n| n == "index");
    let parent_ok = real_index_root.parent() == Some(real_home.as_path());
    if !basename_ok || !parent_ok {
        return Err(format!(
            "Refusing to clear unsafe index path: {}",
            real_index_root.display()
        ));
    }

    let entries = std::fs::read_dir(&real_index_root)
        .map(Iterator::count)
        .unwrap_or(0);
    std::fs::remove_dir_all(&real_index_root).map_err(|e| e.to_string())?;

    Ok(ClearIndexResult {
        path: index_root,
        cleared: true,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn loc(base: &Path) -> CacheLocation {
        CacheLocation {
            base_dir: Some(base.to_path_buf()),
            git_ref: None,
        }
    }

    fn cfile(path: &str, content: &str) -> CacheFile {
        CacheFile {
            path: path.to_string(),
            content: content.as_bytes().to_vec(),
        }
    }

    // --- resolve_cache_dir ---

    #[test]
    fn cache_dir_is_under_index() {
        let base = Path::new("/some/home/.csp");
        let dir = resolve_cache_dir("/repo", &[ContentType::Code], &loc(base));
        assert!(dir.starts_with(base.join("index")));
    }

    #[test]
    fn cache_dir_deterministic() {
        let base = Path::new("/h/.csp");
        let a = resolve_cache_dir("/repo", &[ContentType::Code], &loc(base));
        let b = resolve_cache_dir("/repo", &[ContentType::Code], &loc(base));
        assert_eq!(a, b);
    }

    #[test]
    fn cache_dir_insensitive_to_content_order() {
        let base = Path::new("/h/.csp");
        let a = resolve_cache_dir("/repo", &[ContentType::Code, ContentType::Docs], &loc(base));
        let b = resolve_cache_dir("/repo", &[ContentType::Docs, ContentType::Code], &loc(base));
        assert_eq!(a, b);
    }

    #[test]
    fn cache_dir_differs_by_content() {
        let base = Path::new("/h/.csp");
        let a = resolve_cache_dir("/repo", &[ContentType::Code], &loc(base));
        let b = resolve_cache_dir("/repo", &[ContentType::Code, ContentType::Docs], &loc(base));
        assert_ne!(a, b);
    }

    #[test]
    fn cache_dir_differs_by_source() {
        let base = Path::new("/h/.csp");
        let a = resolve_cache_dir("/repo-a", &[ContentType::Code], &loc(base));
        let b = resolve_cache_dir("/repo-b", &[ContentType::Code], &loc(base));
        assert_ne!(a, b);
    }

    #[test]
    fn cache_dir_differs_by_ref() {
        let base = Path::new("/h/.csp");
        let mut a_loc = loc(base);
        a_loc.git_ref = Some("main".to_string());
        let mut b_loc = loc(base);
        b_loc.git_ref = Some("dev".to_string());
        let a = resolve_cache_dir("https://x/r.git", &[ContentType::Code], &a_loc);
        let b = resolve_cache_dir("https://x/r.git", &[ContentType::Code], &b_loc);
        assert_ne!(a, b);
    }

    // --- compute_content_hash ---

    #[test]
    fn content_hash_order_independent() {
        let a = compute_content_hash(&[cfile("a.ts", "one"), cfile("b.ts", "two")]);
        let b = compute_content_hash(&[cfile("b.ts", "two"), cfile("a.ts", "one")]);
        assert_eq!(a, b);
    }

    #[test]
    fn content_hash_changes_with_content() {
        let a = compute_content_hash(&[cfile("a.ts", "hello")]);
        let b = compute_content_hash(&[cfile("a.ts", "hellp")]);
        assert_ne!(a, b);
    }

    #[test]
    fn content_hash_changes_with_path() {
        let a = compute_content_hash(&[cfile("a.ts", "x")]);
        let b = compute_content_hash(&[cfile("b.ts", "x")]);
        assert_ne!(a, b);
    }

    #[test]
    fn content_hash_bytes_equal_string() {
        let a = compute_content_hash(&[cfile("a.ts", "abc")]);
        let b = compute_content_hash(&[CacheFile {
            path: "a.ts".to_string(),
            content: vec![0x61, 0x62, 0x63],
        }]);
        assert_eq!(a, b);
    }

    #[test]
    fn content_hash_is_hex_sha256() {
        let h = compute_content_hash(&[cfile("a.ts", "x")]);
        assert_eq!(h.len(), 64);
        assert!(h
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    // --- resolve_index_root ---

    #[test]
    fn index_root_is_home_index() {
        let base = Path::new("/h/.csp");
        assert_eq!(resolve_index_root(&loc(base)), base.join("index"));
    }

    #[test]
    fn cache_leaf_lives_under_index_root() {
        let base = Path::new("/h/.csp");
        let root = resolve_index_root(&loc(base));
        let leaf = resolve_cache_dir("/repo", &[ContentType::Code], &loc(base));
        assert!(leaf.starts_with(&root));
    }

    // --- ensure_cache_dir (Unix permissions) ---

    #[cfg(unix)]
    #[test]
    fn ensure_creates_chain_0700_and_tightens() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let leaf = resolve_cache_dir("/repo", &[ContentType::Code], &loc(&base));
        ensure_cache_dir(&leaf, &loc(&base)).unwrap();

        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&leaf), 0o700);
        assert_eq!(mode(&base.join("index")), 0o700);
        assert_eq!(mode(&base), 0o700);

        // Loosen, then re-ensure tightens back.
        std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(base.join("index"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        ensure_cache_dir(&leaf, &loc(&base)).unwrap();
        assert_eq!(mode(&base), 0o700);
        assert_eq!(mode(&base.join("index")), 0o700);
    }

    // --- clear_index_cache ---

    #[test]
    fn clear_removes_index_root_and_counts_entries() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let index_root = resolve_index_root(&loc(&base));
        std::fs::create_dir_all(index_root.join("key-a")).unwrap();
        std::fs::create_dir_all(index_root.join("key-b")).unwrap();
        std::fs::write(index_root.join("key-a/manifest.json"), "{}").unwrap();

        let result = clear_index_cache(&loc(&base)).unwrap();
        assert!(result.cleared);
        assert_eq!(result.entries, 2);
        assert_eq!(result.path, index_root);
        assert!(!index_root.exists());
    }

    #[test]
    fn clear_preserves_savings_and_home() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let index_root = resolve_index_root(&loc(&base));
        std::fs::create_dir_all(index_root.join("key-a")).unwrap();
        let savings = base.join("savings.jsonl");
        std::fs::write(&savings, "{\"call\":\"search\"}\n").unwrap();

        clear_index_cache(&loc(&base)).unwrap();
        assert!(!index_root.exists());
        assert!(savings.exists());
        assert!(base.exists());
    }

    #[test]
    fn clear_reports_missing_root() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let result = clear_index_cache(&loc(&base)).unwrap();
        assert!(!result.cleared);
        assert_eq!(result.entries, 0);
        assert_eq!(result.path, resolve_index_root(&loc(&base)));
    }

    #[cfg(unix)]
    #[test]
    fn clear_refuses_symlink_to_outside_target() {
        use std::os::unix::fs::symlink;
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let victim = tmp.path().join("victim");
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::write(victim.join("precious.txt"), "do not delete").unwrap();
        std::fs::create_dir_all(&base).unwrap();
        symlink(&victim, resolve_index_root(&loc(&base))).unwrap();

        let err = clear_index_cache(&loc(&base)).unwrap_err();
        assert!(err.contains("Refusing to clear unsafe"));
        assert!(victim.join("precious.txt").exists());
    }

    #[cfg(unix)]
    #[test]
    fn clear_refuses_symlink_to_other_index_outside_home() {
        use std::os::unix::fs::symlink;
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let outside_index = tmp.path().join("elsewhere/index");
        std::fs::create_dir_all(&outside_index).unwrap();
        std::fs::write(outside_index.join("precious.txt"), "do not delete").unwrap();
        std::fs::create_dir_all(&base).unwrap();
        symlink(&outside_index, resolve_index_root(&loc(&base))).unwrap();

        let err = clear_index_cache(&loc(&base)).unwrap_err();
        assert!(err.contains("Refusing to clear unsafe"));
        assert!(outside_index.join("precious.txt").exists());
    }
}
