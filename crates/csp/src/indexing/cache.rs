//! Global on-disk index cache location + content hashing. Port of the *pure*
//! pieces of `src/indexing/cache.ts` (T015):
//!
//! - `resolve_cache_dir` — deterministic cache dir for a (source, content, ref) triple.
//! - `resolve_index_root` — `<home>/index`, parent of every cache leaf.
//! - `compute_content_hash` — order-independent sha256 of a file set.
//! - `ensure_cache_dir` — create the `~/.csp → index → leaf` chain with 0700 permissions (NFR-003), tightening any pre-existing directory (Unix).
//! - `clear_index_cache` — safety-guarded removal of the index root only.
//! - `clear_orphan_indexes` — remove entries whose local source path is gone.
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
use crate::utils::is_git_url;

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

/// A cache entry removed by [`clear_orphan_indexes`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrphanIndex {
    /// The removed entry directory (`<home>/index/<key>`).
    pub path: PathBuf,
    /// The local source path the entry's manifest recorded, which no longer exists.
    pub source_id: String,
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

/// Normalize a source identity: URLs (any `scheme://`, or anything `is_git_url`
/// accepts, including scp-style `user@host:path`) are kept
/// verbatim; local paths are made absolute against the current directory and
/// then path-normalized, so `.`, `./repo/../repo`, and `/abs/repo` all key the
/// same entry (mirrors upstream `cache_key`'s `Path.resolve()`). Absolutizing
/// here — rather than only at the CLI edge — is what keeps every caller of
/// `resolve_cache_dir` (CLI, MCP, SDK) on one key.
/// `std::path::absolute` does not touch the filesystem, so a path that does not
/// exist yet still keys deterministically.
///
/// This is *not* byte-identical to the manifest `sourceId`: `from_path` records
/// bare `std::path::absolute(root)`, which keeps `..` segments this function
/// collapses. Anything comparing the two (notably [`source_is_gone`]) must
/// normalize both sides rather than assume they already agree.
pub(crate) fn normalize_source(source: &str) -> String {
    // `is_git_url` is the same classifier `load_or_build_index` uses to pick
    // `from_git`, so every remote spelling it accepts — including non-`git`
    // scp-style ones like `deploy@host:org/repo.git` — stays verbatim instead
    // of being absolutized against the caller's cwd.
    if is_url_scheme(source) || is_git_url(source) {
        return source.to_string();
    }
    let absolute = std::path::absolute(source)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| source.to_string());
    // Only Windows spells its separator `\`. On Unix a backslash is an ordinary
    // filename byte, so rewriting it would fold `/home/u/a\b` onto the unrelated
    // `/home/u/a/b` (and `..\x` into a `..` segment `normalize_posix` then
    // collapses) — two distinct repos sharing one cache leaf, thrashing each
    // other's index on every search.
    #[cfg(windows)]
    let absolute = absolute.replace('\\', "/");
    normalize_posix(&absolute)
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
        update_content_hash(&mut hasher, &file.path, &file.content);
    }
    to_hex(&hasher.finalize())
}

/// Compute the same content hash while reading one file at a time, avoiding
/// retention of every file body for large repositories. Unreadable files are
/// skipped, matching the indexer's existing best-effort collection behavior.
pub(crate) fn compute_content_hash_from_paths(mut files: Vec<(String, PathBuf)>) -> String {
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut hasher = Sha256::new();
    for (relative_path, path) in files {
        let Ok(content) = std::fs::read(path) else {
            continue;
        };
        update_content_hash(&mut hasher, &relative_path, &content);
    }
    to_hex(&hasher.finalize())
}

/// sha256 (hex) of raw bytes — the per-file hash recorded in the index's file
/// manifest for incremental reindexing.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    to_hex(&hasher.finalize())
}

fn update_content_hash(hasher: &mut Sha256, path: &str, content: &[u8]) {
    let len16 = path.encode_utf16().count();
    hasher.update(format!("{len16}:{path}").as_bytes());
    hasher.update(content);
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

/// Resolve the canonical index root the clear commands may delete under.
/// Safety-critical (AC-015): the resolved target must be the direct `index`
/// child of the resolved home, so a symlinked or misconfigured root cannot
/// escalate into a wider delete. `Ok(None)` when no index root exists yet.
fn guarded_index_root(loc: &CacheLocation) -> Result<Option<PathBuf>, String> {
    let home = cache_home(loc);
    let index_root = resolve_index_root(loc);
    if !index_root.exists() {
        return Ok(None);
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
    Ok(Some(real_index_root))
}

/// Remove the cached-index root (`<home>/index`) and report how many entries it
/// held. Deletes *only* the `index` directory (see [`guarded_index_root`]).
pub fn clear_index_cache(loc: &CacheLocation) -> Result<ClearIndexResult, String> {
    let index_root = resolve_index_root(loc);
    let Some(real_index_root) = guarded_index_root(loc)? else {
        return Ok(ClearIndexResult {
            path: index_root,
            cleared: false,
            entries: 0,
        });
    };

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

/// True when `name` has the shape [`resolve_cache_dir`] produces for a cache
/// leaf: the first [`KEY_LENGTH`] lowercase hex chars of a sha256. Mirrors
/// upstream `_clear_orphans`' `_SHA_256_REGEX` guard, so stray directories a
/// user (or another tool) dropped into `~/.csp/index/` are never swept.
fn is_cache_key_name(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|n| {
        n.len() == KEY_LENGTH
            && n.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// The one manifest field the orphan sweep needs. Serde skips every other
/// field (notably the per-file `files` map) without building a value tree.
#[derive(serde::Deserialize)]
struct ManifestSourceId {
    #[serde(rename = "sourceId")]
    source_id: Option<String>,
}

/// Read just the `sourceId` out of `<dir>/manifest.json`, or `None` when the
/// file is absent, unparseable, or records no string source. Deliberately does
/// not go through `read_manifest`: the orphan sweep needs one scalar, while a
/// full parse also deserializes the per-file manifest, which holds an entry for
/// every indexed file.
fn read_manifest_source_id(dir: &Path) -> Option<String> {
    let raw = std::fs::read_to_string(dir.join("manifest.json")).ok()?;
    let manifest: ManifestSourceId = serde_json::from_str(&raw).ok()?;
    let source_id = manifest.source_id?;
    (!source_id.is_empty()).then_some(source_id)
}

/// Whether `source_id` names a path that is known to be gone — a genuine
/// `NotFound`, as opposed to one that merely cannot be reached right now
/// (`PermissionDenied`, a stale network handle, an I/O error). `Path::exists`
/// collapses both into `false`, which would delete live caches for sources
/// under a directory whose permissions temporarily deny traversal. A source
/// below an *unmounted* volume is not distinguishable from a deleted one at
/// this level — its path is simply absent — and is swept, as upstream does.
///
/// Both the recorded spelling and its lexically normalized form must be
/// `NotFound`: `from_path` records `std::path::absolute`, which does **not**
/// collapse `..`, and the kernel resolves `..` component-by-component. So
/// `csp search "q" ../b` records `/cwd/../b`, and deleting `/cwd` alone makes
/// that spelling `NotFound` while `/b` — the actual source — is still there.
/// Requiring both to be absent keeps the sweep conservative: a lexical
/// collapse that crosses a symlink can only ever *keep* a cache entry.
fn source_is_gone(source_id: &str) -> bool {
    fn is_not_found(path: &str) -> bool {
        matches!(
            std::fs::metadata(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound
        )
    }
    if !is_not_found(source_id) {
        return false;
    }
    let normalized = normalize_source(source_id);
    normalized == source_id || is_not_found(&normalized)
}

/// Remove cached indexes whose local source directory no longer exists (port of
/// upstream `_clear_orphans`, semble#243). An entry qualifies only when it sits
/// in a cache-key-shaped directory and its manifest records an absolute, local
/// `sourceId` that is now `NotFound` — git-URL entries, unreadable or malformed
/// manifests, relative `sourceId`s (which would be judged against the caller's
/// cwd), and sources that merely cannot be reached are left untouched. Returns
/// the removed entries, sorted by path. `clear all` deliberately does not
/// include this pass.
pub fn clear_orphan_indexes(loc: &CacheLocation) -> Result<Vec<OrphanIndex>, String> {
    let Some(real_index_root) = guarded_index_root(loc)? else {
        return Ok(Vec::new());
    };

    // Traversal errors surface instead of being skipped: a silently dropped
    // entry would let an incomplete sweep report "no orphans found".
    let mut dirs: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(&real_index_root).map_err(|e| {
        format!(
            "failed to read index root {}: {e}",
            real_index_root.display()
        )
    })? {
        let entry =
            entry.map_err(|e| format!("failed to read {}: {e}", real_index_root.display()))?;
        let file_type = entry
            .file_type()
            .map_err(|e| format!("failed to read {}: {e}", entry.path().display()))?;
        // Real directories only: a symlinked entry could point outside the cache.
        if file_type.is_dir() {
            dirs.push(entry.path());
        }
    }
    dirs.sort();

    let mut removed = Vec::new();
    for dir in dirs {
        if !dir.file_name().is_some_and(is_cache_key_name) {
            continue;
        }
        let Some(source_id) = read_manifest_source_id(&dir) else {
            continue;
        };
        // Git entries are re-rooted to their URL by `from_git`, so the URL check
        // is what keeps a remote index (whose source is never a local path) out
        // of the sweep.
        if is_git_url(&source_id) {
            continue;
        }
        // `from_path` records `std::path::absolute(root)`, so a local entry's
        // source is always absolute. Anything else cannot be resolved without
        // guessing a base directory.
        if !Path::new(&source_id).is_absolute() || !source_is_gone(&source_id) {
            continue;
        }
        std::fs::remove_dir_all(&dir)
            .map_err(|e| format!("failed to remove {}: {e}", dir.display()))?;
        removed.push(OrphanIndex {
            path: dir,
            source_id,
        });
    }
    Ok(removed)
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
    fn cache_dir_relative_and_absolute_source_share_a_key() {
        let base = Path::new("/h/.csp");
        let cwd = std::env::current_dir().unwrap();
        let dot = resolve_cache_dir(".", &[ContentType::Code], &loc(base));
        let abs = resolve_cache_dir(&cwd.to_string_lossy(), &[ContentType::Code], &loc(base));
        assert_eq!(dot, abs);

        // Lexical detours resolve to the same absolute form.
        let sub = cwd.join("sub");
        let detour = resolve_cache_dir("./sub/../sub", &[ContentType::Code], &loc(base));
        let direct = resolve_cache_dir(&sub.to_string_lossy(), &[ContentType::Code], &loc(base));
        assert_eq!(detour, direct);
        assert_ne!(dot, detour);
    }

    #[test]
    fn cache_dir_relative_sources_key_by_their_absolute_form() {
        // Two different relative names must not collide, and each must equal
        // the key its absolute form produces — this is what makes `csp search`
        // from two repos with the default `.` land in two cache entries.
        let base = Path::new("/h/.csp");
        let cwd = std::env::current_dir().unwrap();
        let a = resolve_cache_dir("repo-a", &[ContentType::Code], &loc(base));
        let b = resolve_cache_dir("repo-b", &[ContentType::Code], &loc(base));
        assert_ne!(a, b);
        let a_abs = cwd.join("repo-a");
        assert_eq!(
            a,
            resolve_cache_dir(&a_abs.to_string_lossy(), &[ContentType::Code], &loc(base))
        );
    }

    /// On Unix `\\` is an ordinary filename byte, so two distinct repos must not
    /// collapse onto one cache leaf.
    #[cfg(unix)]
    #[test]
    fn cache_dir_keeps_unix_backslash_paths_distinct() {
        let base = Path::new("/h/.csp");
        let escaped = resolve_cache_dir("/repos/a\\b", &[ContentType::Code], &loc(base));
        let nested = resolve_cache_dir("/repos/a/b", &[ContentType::Code], &loc(base));
        assert_ne!(escaped, nested);
        // A `..` must not be synthesizable out of a backslash either.
        let literal = resolve_cache_dir("/repos/x/..\\y", &[ContentType::Code], &loc(base));
        let traversed = resolve_cache_dir("/repos/y", &[ContentType::Code], &loc(base));
        assert_ne!(literal, traversed);
    }

    /// Every remote spelling `is_git_url` accepts must stay verbatim — not only
    /// `git@…` — or the same remote keys differently from each cwd.
    #[test]
    fn normalize_source_keeps_scp_remotes_verbatim() {
        for remote in [
            "deploy@host:org/repo.git",
            "user@host:repo",
            "git@github.com:x/r.git",
        ] {
            assert_eq!(normalize_source(remote), remote);
        }
        // `user@host:/abs` is not scp syntax; it is a local path and gets keyed as one.
        let local = normalize_source("user@host:/abs");
        assert_ne!(local, "user@host:/abs");
        assert!(Path::new(&local).is_absolute());
    }

    #[test]
    fn cache_dir_keeps_git_urls_verbatim() {
        let base = Path::new("/h/.csp");
        let a = resolve_cache_dir("https://x/r.git", &[ContentType::Code], &loc(base));
        let b = resolve_cache_dir("git@github.com:x/r.git", &[ContentType::Code], &loc(base));
        assert_ne!(a, b);
        // Re-resolving must not absolutize a URL against the cwd.
        assert_eq!(
            a,
            resolve_cache_dir("https://x/r.git", &[ContentType::Code], &loc(base))
        );
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
    fn streamed_content_hash_matches_in_memory_hash() {
        let dir = tempdir().unwrap();
        let a_path = dir.path().join("a.ts");
        let b_path = dir.path().join("b.ts");
        std::fs::write(&a_path, "one").unwrap();
        std::fs::write(&b_path, "two").unwrap();

        let streamed = compute_content_hash_from_paths(vec![
            ("b.ts".to_string(), b_path),
            ("a.ts".to_string(), a_path),
        ]);
        let in_memory = compute_content_hash(&[cfile("a.ts", "one"), cfile("b.ts", "two")]);
        assert_eq!(streamed, in_memory);
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

    // --- clear_orphan_indexes ---

    fn manifest_json(source_id: &str, content: &[ContentType]) -> String {
        let manifest = crate::indexing::index::IndexManifest {
            schema_version: crate::indexing::index::INDEX_SCHEMA_VERSION,
            content_hash: "hash".to_string(),
            source_id: Some(source_id.to_string()),
            content: content.to_vec(),
            model_id: "model".to_string(),
            model_kind: Some("stub".to_string()),
            chunk_size: Some(750),
            files: Default::default(),
        };
        serde_json::to_string(&manifest).unwrap()
    }

    /// Write a cache entry keyed exactly as `load_or_build_index` would for
    /// `source_id`, with a manifest recording that source.
    fn write_entry(source_id: &str, entry_loc: &CacheLocation) -> PathBuf {
        write_entry_keyed(source_id, source_id, entry_loc)
    }

    /// Write a cache entry whose directory is keyed on `key_source` but whose
    /// manifest records `manifest_source`. Production can split the two: the key
    /// also folds in `CacheLocation::git_ref`, which the manifest never records,
    /// and `from_path` keeps the `..` segments `normalize_source` collapses.
    fn write_entry_keyed(
        key_source: &str,
        manifest_source: &str,
        entry_loc: &CacheLocation,
    ) -> PathBuf {
        let dir = resolve_cache_dir(key_source, &[ContentType::Code], entry_loc);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("manifest.json"),
            manifest_json(manifest_source, &[ContentType::Code]),
        )
        .unwrap();
        dir
    }

    #[test]
    fn orphans_removes_entry_whose_source_is_gone() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let gone = tmp.path().join("gone-repo");
        let dir = write_entry(&gone.to_string_lossy(), &loc(&base));

        let removed = clear_orphan_indexes(&loc(&base)).unwrap();
        assert_eq!(
            removed,
            vec![OrphanIndex {
                path: std::fs::canonicalize(&base)
                    .unwrap()
                    .join("index")
                    .join(dir.file_name().unwrap()),
                source_id: gone.to_string_lossy().into_owned(),
            }]
        );
        assert!(!dir.exists());
        assert!(resolve_index_root(&loc(&base)).exists());
    }

    /// Regression test for a sweep that filters on the entry directory's shape
    /// rather than on a re-derived key: the manifest's `sourceId` is the only
    /// thing consulted, so an entry stays sweepable however its key was spelled.
    #[test]
    fn orphans_removes_entry_keyed_by_a_relative_source() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let gone = tmp.path().join("gone-repo");
        let dir = write_entry_keyed(".", &gone.to_string_lossy(), &loc(&base));

        let removed = clear_orphan_indexes(&loc(&base)).unwrap();
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].source_id, gone.to_string_lossy());
        assert!(!dir.exists());
    }

    /// A local entry built with `--ref` carries the ref in its key but not in
    /// its manifest, so it must still be swept once its source is gone.
    #[test]
    fn orphans_removes_local_entry_built_with_a_ref() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let gone = tmp.path().join("gone-repo");
        let mut ref_loc = loc(&base);
        ref_loc.git_ref = Some("main".to_string());
        let dir = write_entry(&gone.to_string_lossy(), &ref_loc);

        assert_eq!(clear_orphan_indexes(&loc(&base)).unwrap().len(), 1);
        assert!(!dir.exists());
    }

    #[test]
    fn orphans_keeps_entry_whose_source_exists() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let live = tmp.path().join("live-repo");
        std::fs::create_dir_all(&live).unwrap();
        let dir = write_entry(&live.to_string_lossy(), &loc(&base));

        assert!(clear_orphan_indexes(&loc(&base)).unwrap().is_empty());
        assert!(dir.join("manifest.json").exists());
    }

    #[test]
    fn orphans_keeps_git_entry() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let mut git_loc = loc(&base);
        git_loc.git_ref = Some("main".to_string());
        let with_ref = write_entry("https://github.com/x/y.git", &git_loc);
        // A ref-less git entry reproduces its own key exactly (`from_git`
        // re-roots the manifest to the URL), so `is_git_url` is the only thing
        // keeping it out of the sweep.
        let no_ref = write_entry("https://github.com/x/z.git", &loc(&base));

        assert!(clear_orphan_indexes(&loc(&base)).unwrap().is_empty());
        assert!(with_ref.join("manifest.json").exists());
        assert!(no_ref.join("manifest.json").exists());
    }

    /// A relative `sourceId` would be resolved against the caller's cwd, so it
    /// is never trusted — `from_path` always records an absolute path.
    #[test]
    fn orphans_keeps_entry_with_a_relative_source_id() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let dir = write_entry("gone-repo", &loc(&base));

        assert!(clear_orphan_indexes(&loc(&base)).unwrap().is_empty());
        assert!(dir.join("manifest.json").exists());
    }

    /// Only a directory named like a cache key is a csp cache entry; anything
    /// else under `~/.csp/index/` belongs to someone else.
    #[test]
    fn orphans_skips_dir_that_is_not_a_cache_key() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let gone = tmp.path().join("gone-repo");
        // Right length, wrong alphabet — and a plain name.
        for name in ["0123456789ABCDEF0123456789abcdef", "notes"] {
            let dir = resolve_index_root(&loc(&base)).join(name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(
                dir.join("manifest.json"),
                manifest_json(&gone.to_string_lossy(), &[ContentType::Code]),
            )
            .unwrap();
        }

        assert!(clear_orphan_indexes(&loc(&base)).unwrap().is_empty());
        assert!(resolve_index_root(&loc(&base)).join("notes").exists());
    }

    #[test]
    fn orphans_skips_malformed_or_missing_manifest() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let root = resolve_index_root(&loc(&base));
        // Key-shaped names, so the manifest read is what rejects these.
        let malformed = root.join("0123456789abcdef0123456789abcdef");
        std::fs::create_dir_all(&malformed).unwrap();
        std::fs::write(malformed.join("manifest.json"), "not json").unwrap();
        let missing = root.join("fedcba9876543210fedcba9876543210");
        std::fs::create_dir_all(&missing).unwrap();
        // A manifest that parses but records no source.
        let sourceless = root.join("00000000000000000000000000000000");
        std::fs::create_dir_all(&sourceless).unwrap();
        std::fs::write(sourceless.join("manifest.json"), r#"{"sourceId":""}"#).unwrap();
        std::fs::write(root.join("stray-file"), "x").unwrap();

        assert!(clear_orphan_indexes(&loc(&base)).unwrap().is_empty());
        assert!(malformed.exists());
        assert!(missing.exists());
        assert!(sourceless.exists());
        assert!(root.join("stray-file").exists());
    }

    /// A source that cannot be stat'd (an unreadable parent, a stale handle) is
    /// not the same as a source that was deleted — the cache must survive it.
    #[cfg(unix)]
    #[test]
    fn orphans_keeps_entry_whose_source_is_unreachable() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let vault = tmp.path().join("vault");
        let live = vault.join("repo");
        std::fs::create_dir_all(&live).unwrap();
        let dir = write_entry(&live.to_string_lossy(), &loc(&base));
        std::fs::set_permissions(&vault, std::fs::Permissions::from_mode(0o000)).unwrap();

        // Running as root ignores the permission bits; skip rather than assert.
        let blocked = std::fs::metadata(&live).is_err();
        let swept = clear_orphan_indexes(&loc(&base)).unwrap();
        std::fs::set_permissions(&vault, std::fs::Permissions::from_mode(0o700)).unwrap();
        if blocked {
            assert!(swept.is_empty());
            assert!(dir.join("manifest.json").exists());
        }
    }

    /// `from_path` records `std::path::absolute`, which keeps `..` segments, and
    /// the kernel resolves `..` component-by-component. Deleting only the
    /// intermediate directory must not make the still-present source look gone.
    #[test]
    fn orphans_keeps_entry_whose_source_id_traverses_a_deleted_parent() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let via = tmp.path().join("via");
        let live = tmp.path().join("live-repo");
        std::fs::create_dir_all(&via).unwrap();
        std::fs::create_dir_all(&live).unwrap();
        // The shape `csp search "q" ../live-repo` records from inside `via`.
        let recorded = via.join("..").join("live-repo");
        let dir = write_entry(&recorded.to_string_lossy(), &loc(&base));
        std::fs::remove_dir_all(&via).unwrap();

        assert!(std::fs::metadata(&recorded).is_err(), "precondition");
        assert!(clear_orphan_indexes(&loc(&base)).unwrap().is_empty());
        assert!(dir.join("manifest.json").exists());
    }

    #[test]
    fn orphans_reports_nothing_without_index_root() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        assert!(clear_orphan_indexes(&loc(&base)).unwrap().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn orphans_refuses_symlinked_index_root() {
        use std::os::unix::fs::symlink;
        let tmp = tempdir().unwrap();
        let base = tmp.path().join(".csp");
        let victim = tmp.path().join("victim");
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::create_dir_all(&base).unwrap();
        symlink(&victim, resolve_index_root(&loc(&base))).unwrap();

        let err = clear_orphan_indexes(&loc(&base)).unwrap_err();
        assert!(err.contains("Refusing to clear unsafe"));
    }
}
