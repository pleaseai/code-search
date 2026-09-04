//! Per-file character counts feeding the `file_chars` side of token-savings
//! telemetry (`crate::stats`).
//!
//! Deliberate divergence from upstream semble, which recomputes every indexed
//! file's size eagerly in `SembleIndex.__init__`: a local source root is read
//! lazily, per result, with a memo — only the handful of files a query actually
//! returns is touched. Git sources still capture eagerly, at clone time.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// UTF-16 character counts per repo-relative file path, resolved eagerly
/// (captured) or lazily (read from a local root on demand).
#[derive(Debug, Default)]
pub struct FileSizes {
    /// Sizes captured while the source tree was on disk (git clones: the temp
    /// checkout is gone by search time).
    captured: HashMap<String, u64>,
    /// Local source root read on demand for paths not in `captured`.
    lazy_root: Option<PathBuf>,
    /// Memo of lazily read sizes; `Mutex` because `CspIndex` is shared as
    /// `Arc<CspIndex>` across MCP calls.
    memo: Mutex<HashMap<String, u64>>,
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

    /// Sizes read on demand from a still-present local source root.
    pub fn lazy(root: PathBuf) -> Self {
        Self {
            lazy_root: Some(root),
            ..Self::default()
        }
    }

    /// Character count for `file_path`: captured → memo → read from the lazy
    /// root (memoized on success). `None` when unavailable or unreadable.
    pub fn get(&self, file_path: &str) -> Option<u64> {
        if let Some(size) = self.captured.get(file_path) {
            return Some(*size);
        }
        let root = self.lazy_root.as_deref()?;
        if let Some(size) = self.lock_memo().get(file_path) {
            return Some(*size);
        }
        let size = read_file_chars(root, file_path)?;
        self.lock_memo().insert(file_path.to_string(), size);
        Some(size)
    }

    /// `true` when sizes can be produced at all. Prefer this over inspecting
    /// the map: a lazy root reports available before anything has been read.
    pub fn is_available(&self) -> bool {
        !self.captured.is_empty() || self.lazy_root.is_some()
    }

    fn lock_memo(&self) -> std::sync::MutexGuard<'_, HashMap<String, u64>> {
        self.memo.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// UTF-16 character count of the repo-relative `file_path` under `root`, or
/// `None` when it cannot be read. UTF-16 keeps it consistent with
/// `stats::save_search_stats`'s snippet accounting.
///
/// Chunk paths are repo-relative by construction; a path that is absolute or
/// escapes `root` via `..` can only come from a tampered on-disk index, so it is
/// skipped rather than resolved (path traversal guard — a deliberate addition
/// over upstream, which joins the path unchecked). Only regular files are read:
/// the file walker never follows symlinks, and a path that has since become a
/// symlink, FIFO, or device must not be able to redirect or stall the read.
pub(crate) fn read_file_chars(root: &Path, file_path: &str) -> Option<u64> {
    let rel = Path::new(file_path);
    if !is_safe_relative_path(rel) {
        return None;
    }
    let full = root.join(rel);
    let is_regular_file = std::fs::symlink_metadata(&full)
        .map(|m| m.is_file())
        .unwrap_or(false);
    if !is_regular_file {
        return None;
    }
    let text = std::fs::read_to_string(&full).ok()?;
    Some(text.encode_utf16().count() as u64)
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
        #[cfg(unix)]
        std::os::unix::fs::symlink(outer.path().join("secret.txt"), root.join("link.ts")).unwrap();
        let abs = root.join("real.ts").to_string_lossy().into_owned();
        let sizes = FileSizes::lazy(root.clone());

        assert_eq!(sizes.get("../secret.txt"), None);
        assert_eq!(sizes.get(&abs), None);
        assert_eq!(sizes.get("missing.ts"), None);
        #[cfg(unix)]
        assert_eq!(sizes.get("link.ts"), None);
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
