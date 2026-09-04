//! Per-file character counts feeding the `file_chars` side of token-savings
//! telemetry (`crate::stats`).
//!
//! Deliberate divergence from upstream semble, which recomputes every indexed
//! file's size eagerly in `SembleIndex.__init__`: a local source root is read
//! lazily, per result, with a memo — only the handful of files a query actually
//! returns is touched. Git sources still capture eagerly, at clone time.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::indexing::files::{get_max_file_bytes, DEFAULT_MAX_FILE_BYTES};

/// Upper bound on the buffer `read_file_chars` pre-allocates. The read itself is
/// still capped at the index ceiling; this only stops a raised
/// `CSP_MAX_FILE_BYTES` from turning a single search result into a multi-GB
/// `Vec::with_capacity` before a byte has been read.
const READ_PREALLOC_CAP: u64 = DEFAULT_MAX_FILE_BYTES;

/// UTF-16 character counts per repo-relative file path, resolved eagerly
/// (captured) or lazily (read from a local root on demand).
#[derive(Debug, Default)]
pub struct FileSizes {
    /// Sizes captured while the source tree was on disk (git clones: the temp
    /// checkout is gone by search time).
    captured: HashMap<String, u64>,
    /// Local source root read on demand for paths not in `captured`.
    lazy_root: Option<PathBuf>,
    /// Ceiling for the lazy read, resolved once when the root is set (rather
    /// than per result). `None` only when there is no lazy root to read from.
    lazy_max_bytes: Option<u64>,
    /// Memo of lazily resolved sizes, negatives included — a path that cannot
    /// be read must not re-pay the syscalls (and, for a file the indexer
    /// accepted, a full read) on every later query. `Mutex` because `CspIndex`
    /// is shared as `Arc<CspIndex>` across MCP calls.
    memo: Mutex<HashMap<String, Option<u64>>>,
}

impl FileSizes {
    /// No sizes available — telemetry records `file_chars` as 0.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Sizes already read off a source tree that is no longer available.
    pub fn captured(sizes: HashMap<String, u64>) -> Self {
        Self {
            captured: sizes,
            ..Self::default()
        }
    }

    /// Sizes read on demand from a still-present local source root. The root
    /// is canonicalized once here so each lookup's containment check is a plain
    /// prefix comparison; a root that cannot be canonicalized is kept as-is and
    /// every lookup then fails containment, which is the safe outcome.
    pub fn lazy(root: PathBuf) -> Self {
        Self::lazy_with_limit(root, get_max_file_bytes())
    }

    /// [`FileSizes::lazy`] with the read ceiling pinned instead of resolved from
    /// `CSP_MAX_FILE_BYTES`, so a caller that pinned
    /// `CreateIndexOptions::max_file_bytes` (and a test) can size the reader to
    /// the same limit the index was actually built with.
    pub fn lazy_with_limit(root: PathBuf, max_file_bytes: u64) -> Self {
        let root = root.canonicalize().unwrap_or(root);
        Self {
            lazy_root: Some(root),
            lazy_max_bytes: Some(max_file_bytes),
            ..Self::default()
        }
    }

    /// Character count for `file_path`: captured → memo → read from the lazy
    /// root. `None` when unavailable or unreadable; both outcomes are memoized,
    /// so an unreadable path costs one read attempt per index, not one per
    /// query. The memo lock is released across the read so concurrent lookups
    /// of different files don't serialize.
    pub fn get(&self, file_path: &str) -> Option<u64> {
        if let Some(size) = self.captured.get(file_path) {
            return Some(*size);
        }
        let root = self.lazy_root.as_deref()?;
        if let Some(size) = self.lock_memo().get(file_path) {
            return *size;
        }
        let size = read_file_chars(
            root,
            file_path,
            self.lazy_max_bytes.unwrap_or_else(get_max_file_bytes),
        );
        self.lock_memo().insert(file_path.to_string(), size);
        size
    }

    /// `true` when sizes can be produced at all. Prefer this over inspecting
    /// the map: a lazy root reports available before anything has been read.
    pub fn is_available(&self) -> bool {
        !self.captured.is_empty() || self.lazy_root.is_some()
    }

    fn lock_memo(&self) -> std::sync::MutexGuard<'_, HashMap<String, Option<u64>>> {
        self.memo.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// UTF-16 character count of the repo-relative `file_path` under `root`, or
/// `None` when it cannot be read. `root` must already be canonical (see
/// [`FileSizes::lazy`]); the containment check below is a prefix comparison
/// against it. UTF-16 keeps it consistent with `stats::save_search_stats`'s
/// snippet accounting.
///
/// Chunk paths are repo-relative by construction; a path that is absolute or
/// escapes `root` via `..` can only come from a tampered on-disk index, so it is
/// skipped rather than resolved (path traversal guard — a deliberate addition
/// over upstream, which joins the path unchecked). Only regular files are read:
/// the file walker never follows symlinks, and a path that has since become a
/// symlink, FIFO, or device must not be able to redirect or stall the read.
///
/// The read is bounded by `max_file_bytes`, the ceiling the index was built
/// with (see [`FileSizes::lazy_with_limit`]) — lazily, this runs inside a live search,
/// so a file that has grown past the indexing limit since it was chunked must
/// not be slurped whole on the query path. Decoding is lossy, matching the
/// indexer (`String::from_utf8_lossy`) and upstream `read_file_text`'s
/// `errors="replace"`: a file with invalid UTF-8 still gets indexed, so it must
/// still be sized instead of silently contributing 0 `file_chars`.
pub(crate) fn read_file_chars(root: &Path, file_path: &str, max_file_bytes: u64) -> Option<u64> {
    let rel = Path::new(file_path);
    if !is_safe_relative_path(rel) {
        return None;
    }
    let full = root.join(rel);
    // Reject a symlink at the leaf (the walker never indexes one) and, via
    // canonicalization, a symlinked intermediate directory that would resolve
    // the read outside `root`.
    if std::fs::symlink_metadata(&full).ok()?.is_symlink() {
        return None;
    }
    let canonical = full.canonicalize().ok()?;
    if !canonical.starts_with(root) {
        return None;
    }
    // Reject a non-regular file *before* opening it: `open(2)` on a FIFO
    // blocks until a writer shows up, which would stall the search path, and
    // opening a device node can have side effects. The fstat below re-checks
    // the opened handle so the regular-file and size checks apply to what is
    // actually read, and the read itself is capped.
    if !std::fs::symlink_metadata(&canonical).ok()?.is_file() {
        return None;
    }
    // Residual race: `canonicalize` and `File::open` are separate path walks,
    // so a writer swapping a parent directory for a symlink in between can make
    // the open follow it to a regular file outside `root`. Closing that needs a
    // descriptor-relative no-follow walk (`openat` + `O_NOFOLLOW` per component),
    // which `std` does not expose portably. The exposure is a UTF-16 length of
    // that file written to the user's own `savings.jsonl`, never its content, by
    // a local writer who already controls the indexed tree.
    let file = std::fs::File::open(&canonical).ok()?;
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > max_file_bytes {
        return None;
    }
    // Reserve what the file claims, but never more than the default ceiling: a
    // raised `CSP_MAX_FILE_BYTES` must not let one search result reserve
    // gigabytes up front. `read_to_end` grows past it if the file really is
    // that large.
    let mut bytes = Vec::with_capacity(meta.len().min(READ_PREALLOC_CAP) as usize);
    file.take(max_file_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > max_file_bytes {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).encode_utf16().count() as u64)
}

/// `true` when `path` is relative and contains no `..` or root component, so
/// joining it onto an index root cannot resolve outside that root.
fn is_safe_relative_path(path: &Path) -> bool {
    use std::path::Component;
    !path.is_absolute()
        && !path.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn lazy_reads_and_memoizes_regular_files() {
        let root = tempdir().unwrap();
        std::fs::write(root.path().join("a.ts"), "abcd").unwrap();
        let sizes = FileSizes::lazy(root.path().to_path_buf());

        assert!(sizes.is_available());
        assert_eq!(sizes.get("a.ts"), Some(4));
        // Memoized: the value survives the file going away.
        std::fs::remove_file(root.path().join("a.ts")).unwrap();
        assert_eq!(sizes.get("a.ts"), Some(4));
    }

    #[test]
    fn lazy_returns_none_for_unreadable_paths() {
        let outer = tempdir().unwrap();
        let root = outer.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::write(outer.path().join("secret.txt"), "top secret").unwrap();
        std::fs::write(root.join("real.ts"), "abcd").unwrap();
        std::fs::create_dir(root.join("dir.ts")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(outer.path().join("secret.txt"), root.join("link.ts")).unwrap();
        let abs = root.join("real.ts").to_string_lossy().into_owned();
        let sizes = FileSizes::lazy(root.clone());

        assert_eq!(sizes.get("../secret.txt"), None);
        assert_eq!(sizes.get(&abs), None);
        assert_eq!(sizes.get("missing.ts"), None);
        assert_eq!(sizes.get("dir.ts"), None);
        #[cfg(unix)]
        assert_eq!(sizes.get("link.ts"), None);
    }

    #[cfg(unix)]
    #[test]
    fn lazy_rejects_symlinked_intermediate_directory() {
        let outer = tempdir().unwrap();
        let root = outer.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        let outside = outer.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("leak.ts"), "top secret").unwrap();
        // `repo/vendor` -> `../outside`: the leaf is a regular file, but the
        // path only reaches it through a symlinked directory.
        std::os::unix::fs::symlink(&outside, root.join("vendor")).unwrap();

        let sizes = FileSizes::lazy(root);
        assert_eq!(sizes.get("vendor/leak.ts"), None);
    }

    #[cfg(unix)]
    #[test]
    fn lazy_rejects_fifo_without_blocking() {
        let root = tempdir().unwrap();
        let fifo = root.path().join("pipe.ts");
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap();
        assert!(status.success());

        // A FIFO with no writer would block `File::open` forever; the
        // pre-open regular-file check must skip it instead.
        let sizes = FileSizes::lazy(root.path().to_path_buf());
        assert_eq!(sizes.get("pipe.ts"), None);
    }

    #[test]
    fn lazy_memoizes_misses_so_they_are_read_once() {
        let root = tempdir().unwrap();
        let sizes = FileSizes::lazy(root.path().to_path_buf());

        assert_eq!(sizes.get("later.ts"), None);
        // The miss is cached: a file appearing afterwards does not resurrect it,
        // which is what proves no second read was attempted.
        std::fs::write(root.path().join("later.ts"), "abcd").unwrap();
        assert_eq!(sizes.get("later.ts"), None);
    }

    #[test]
    fn lazy_sizes_non_utf8_files_lossily_like_the_indexer() {
        let root = tempdir().unwrap();
        // Latin-1 byte: `create_index_from_path` decodes it lossily and indexes
        // the file, so sizing must not reject it.
        std::fs::write(root.path().join("legacy.js"), b"ab\xffcd").unwrap();
        let sizes = FileSizes::lazy(root.path().to_path_buf());

        assert_eq!(sizes.get("legacy.js"), Some(5));
    }

    #[test]
    fn lazy_skips_files_larger_than_the_indexing_ceiling() {
        // The ceiling is pinned, not read from the ambient `CSP_MAX_FILE_BYTES`:
        // the fixture size must not depend on the environment the suite runs in.
        const LIMIT: u64 = 1_024;
        let root = tempdir().unwrap();
        std::fs::write(root.path().join("grown.ts"), vec![b'a'; LIMIT as usize + 1]).unwrap();
        std::fs::write(root.path().join("fits.ts"), vec![b'a'; LIMIT as usize]).unwrap();
        let sizes = FileSizes::lazy_with_limit(root.path().to_path_buf(), LIMIT);

        assert_eq!(sizes.get("grown.ts"), None);
        assert_eq!(sizes.get("fits.ts"), Some(LIMIT));
    }

    #[test]
    fn captured_serves_known_paths_only() {
        let sizes = FileSizes::captured([("a.ts".to_string(), 7u64)].into_iter().collect());

        assert!(sizes.is_available());
        assert_eq!(sizes.get("a.ts"), Some(7));
        assert_eq!(sizes.get("b.ts"), None);
    }

    #[test]
    fn empty_is_not_available() {
        let sizes = FileSizes::empty();

        assert!(!sizes.is_available());
        assert_eq!(sizes.get("a.ts"), None);
    }
}
