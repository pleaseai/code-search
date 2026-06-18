//! Gitignore-aware file walking. Port of `src/indexing/file-walker.ts`
//! (← semble `index/file_walker.py`).
//!
//! Uses the `ignore` crate's `Gitignore` matcher. Its `Match::{None, Ignore,
//! Whitelist}` maps onto the npm `ignore` package's `{ignored, unignored}`
//! result the upstream relied on. The negation-with-extension "bypass" (`found`)
//! is reproduced with per-pattern matchers, exactly as the TS port does.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use ignore::Match;

/// Default directories always ignored when walking (gitignore directory
/// semantics via the trailing `/`). The Python original uses `.semble/`; csp
/// uses `.csp/`.
pub const DEFAULT_IGNORED_DIRS: &[&str] = &[
    ".git/",
    ".hg/",
    ".svn/",
    "__pycache__/",
    "node_modules/",
    ".venv/",
    "venv/",
    ".tox/",
    ".mypy_cache/",
    ".pytest_cache/",
    ".ruff_cache/",
    ".cache/",
    ".csp/",
    ".next/",
    "dist/",
    "build/",
    ".eggs/",
];

/// A single parsed ignore pattern (in source order).
pub struct ParsedPattern {
    /// Pattern text without the leading `!`.
    pub pattern: String,
    /// Whether the source line began with `!`.
    pub negated: bool,
    /// Whether the pattern (trailing `/` stripped) has a file-extension suffix.
    pub has_ext_suffix: bool,
    matcher: Gitignore,
}

/// Merged ignore patterns sourced from one directory's ignore files.
pub struct IgnoreSpec {
    base: PathBuf,
    aggregate: Gitignore,
    pub patterns: Vec<ParsedPattern>,
    /// True when at least one negated pattern has an extension suffix.
    pub has_negated_ext_pattern: bool,
}

/// Result of [`is_ignored`]: `ignored` is the final decision; `found` signals a
/// negation pattern with an extension suffix won, letting the file bypass the
/// extension allowlist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IgnoreCheck {
    pub ignored: bool,
    pub found: bool,
}

/// Node `path.extname`: the final `.ext` of the basename, or `""` for a
/// dotfile / no extension.
fn ext_name(path: &str) -> &str {
    let base = match path.rfind(['/', '\\']) {
        Some(i) => &path[i + 1..],
        None => path,
    };
    match base.rfind('.') {
        Some(0) | None => "",
        Some(i) => &base[i..],
    }
}

fn has_extension_suffix(pattern: &str) -> bool {
    let stripped = pattern.trim_end_matches('/');
    !ext_name(stripped).is_empty()
}

fn build_spec(base: &Path, lines: &[String]) -> IgnoreSpec {
    let mut aggregate = GitignoreBuilder::new(base);
    let mut patterns = Vec::new();

    for raw_line in lines {
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let _ = aggregate.add_line(None, line);

        let negated = trimmed.starts_with('!');
        let pattern = if negated { &trimmed[1..] } else { trimmed };
        if pattern.is_empty() {
            continue;
        }

        let mut pat_builder = GitignoreBuilder::new(base);
        let _ = pat_builder.add_line(None, pattern);
        let matcher = pat_builder.build().unwrap_or_else(|_| Gitignore::empty());

        patterns.push(ParsedPattern {
            pattern: pattern.to_string(),
            negated,
            has_ext_suffix: has_extension_suffix(pattern),
            matcher,
        });
    }

    let has_negated_ext_pattern = patterns.iter().any(|p| p.negated && p.has_ext_suffix);
    let aggregate = aggregate.build().unwrap_or_else(|_| Gitignore::empty());

    IgnoreSpec {
        base: base.to_path_buf(),
        aggregate,
        patterns,
        has_negated_ext_pattern,
    }
}

/// Load `.gitignore` and `.cspignore` from `directory`, merged into one spec,
/// or `None` when neither file is present.
pub fn load_ignore_for_dir(directory: &Path) -> Option<IgnoreSpec> {
    let mut lines: Vec<String> = Vec::new();
    for name in [".gitignore", ".cspignore"] {
        let path = directory.join(name);
        if let Ok(text) = std::fs::read_to_string(&path) {
            for line in text.split('\n') {
                lines.push(line.to_string());
            }
        }
    }
    if lines.is_empty() {
        return None;
    }
    Some(build_spec(directory, &lines))
}

/// Check whether a path is ignored by any of the provided specs (later matches
/// override earlier ones — standard gitignore semantics).
pub fn is_ignored(file_path: &Path, is_dir: bool, specs: &[&IgnoreSpec]) -> IgnoreCheck {
    let mut ignored = false;
    let mut found = false;

    for spec in specs {
        let Ok(rel) = file_path.strip_prefix(&spec.base) else {
            continue;
        };
        if rel.as_os_str().is_empty() {
            continue;
        }

        match spec.aggregate.matched(rel, is_dir) {
            Match::None => continue,
            Match::Ignore(_) => {
                ignored = true;
                found = false;
            }
            Match::Whitelist(_) => {
                if !spec.has_negated_ext_pattern {
                    ignored = false;
                    found = false;
                    continue;
                }
                // Per-pattern walk to determine `found` accurately; last
                // matching pattern wins.
                for pattern in &spec.patterns {
                    if pattern.matcher.matched(rel, is_dir).is_none() {
                        continue;
                    }
                    ignored = !pattern.negated;
                    found = !ignored && pattern.has_ext_suffix;
                }
            }
        }
    }

    IgnoreCheck { ignored, found }
}

fn walk(
    dir: &Path,
    inherited: &[&IgnoreSpec],
    extensions: &HashSet<String>,
    out: &mut Vec<PathBuf>,
) {
    let dir_spec = load_ignore_for_dir(dir);
    let mut specs: Vec<&IgnoreSpec> = inherited.to_vec();
    if let Some(ref spec) = dir_spec {
        specs.push(spec);
    }

    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = read.flatten().collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);

    for entry in entries {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let full = entry.path();
        let is_dir = file_type.is_dir();
        let check = is_ignored(&full, is_dir, &specs);
        if check.ignored {
            continue;
        }

        if is_dir {
            walk(&full, &specs, extensions, out);
        } else if file_type.is_file() {
            let name = entry.file_name();
            let ext = ext_name(&name.to_string_lossy()).to_ascii_lowercase();
            if check.found || extensions.contains(&ext) {
                out.push(full);
            }
        }
    }
}

/// Walk `root`, returning files whose extension is in `extensions`, skipping
/// ignored paths. [`DEFAULT_IGNORED_DIRS`] plus any `extra` patterns are always
/// applied, and `.gitignore`/`.cspignore` files are honoured recursively.
pub fn walk_files(root: &Path, extensions: &[&str], extra: &[&str]) -> Vec<PathBuf> {
    let extensions_set: HashSet<String> =
        extensions.iter().map(|e| e.to_ascii_lowercase()).collect();

    let mut dir_patterns: Vec<String> =
        DEFAULT_IGNORED_DIRS.iter().map(|s| s.to_string()).collect();
    dir_patterns.sort();
    dir_patterns.extend(extra.iter().map(|s| s.to_string()));

    let base_spec = build_spec(root, &dir_patterns);
    let mut out = Vec::new();
    walk(root, &[&base_spec], &extensions_set, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn rel_sorted(root: &Path, paths: &[PathBuf]) -> Vec<String> {
        let mut out: Vec<String> = paths
            .iter()
            .map(|p| {
                p.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace(std::path::MAIN_SEPARATOR, "/")
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn default_ignored_dirs_uses_csp_not_semble() {
        assert!(DEFAULT_IGNORED_DIRS.contains(&".csp/"));
        assert!(!DEFAULT_IGNORED_DIRS.contains(&".semble/"));
        for d in [
            ".git/",
            "node_modules/",
            "dist/",
            "build/",
            ".next/",
            "__pycache__/",
        ] {
            assert!(DEFAULT_IGNORED_DIRS.contains(&d));
        }
    }

    #[test]
    fn yields_ts_files_recursively() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.ts"), "a").unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub/b.ts"), "b").unwrap();
        fs::write(root.join("sub/c.md"), "c").unwrap();
        fs::create_dir(root.join("sub/nested")).unwrap();
        fs::write(root.join("sub/nested/d.ts"), "d").unwrap();

        let results = walk_files(root, &[".ts"], &[]);
        assert_eq!(
            rel_sorted(root, &results),
            ["a.ts", "sub/b.ts", "sub/nested/d.ts"]
        );
    }

    #[test]
    fn always_ignores_git_and_node_modules() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("keep.ts"), "k").unwrap();
        fs::create_dir(root.join(".git")).unwrap();
        fs::write(root.join(".git/hidden.ts"), "h").unwrap();
        fs::create_dir(root.join("node_modules")).unwrap();
        fs::write(root.join("node_modules/pkg.ts"), "p").unwrap();

        let results = walk_files(root, &[".ts"], &[]);
        assert_eq!(rel_sorted(root, &results), ["keep.ts"]);
    }

    #[test]
    fn gitignore_excludes_matching_files() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        fs::write(root.join("foo.log"), "foo").unwrap();
        fs::write(root.join("bar.txt"), "bar").unwrap();

        let results = walk_files(root, &[".log", ".txt"], &[]);
        assert_eq!(rel_sorted(root, &results), ["bar.txt"]);
    }

    #[test]
    fn negation_with_extension_bypasses_extension_filter() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "*.log\n!special.log\n").unwrap();
        fs::write(root.join("foo.log"), "foo").unwrap();
        fs::write(root.join("special.log"), "special").unwrap();
        fs::write(root.join("keep.ts"), "k").unwrap();

        let results = walk_files(root, &[".ts"], &[]);
        assert_eq!(rel_sorted(root, &results), ["keep.ts", "special.log"]);
    }

    #[test]
    fn cspignore_honoured_alongside_gitignore() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join(".gitignore"), "gitignored.ts\n").unwrap();
        fs::write(root.join(".cspignore"), "cspignored.ts\n").unwrap();
        fs::write(root.join("keep.ts"), "k").unwrap();
        fs::write(root.join("gitignored.ts"), "g").unwrap();
        fs::write(root.join("cspignored.ts"), "c").unwrap();

        let results = walk_files(root, &[".ts"], &[]);
        assert_eq!(rel_sorted(root, &results), ["keep.ts"]);
    }

    #[test]
    fn respects_nested_gitignore() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("top.ts"), "t").unwrap();
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub/.gitignore"), "skip.ts\n").unwrap();
        fs::write(root.join("sub/skip.ts"), "s").unwrap();
        fs::write(root.join("sub/keep.ts"), "k").unwrap();

        let results = walk_files(root, &[".ts"], &[]);
        assert_eq!(rel_sorted(root, &results), ["sub/keep.ts", "top.ts"]);
    }

    #[test]
    fn honours_extra_ignore_arg() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("foo.ts"), "f").unwrap();
        fs::write(root.join("bar.ts"), "b").unwrap();

        let results = walk_files(root, &[".ts"], &["foo.ts"]);
        assert_eq!(rel_sorted(root, &results), ["bar.ts"]);
    }

    #[test]
    fn filters_by_extension_case_insensitive() {
        let dir = tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("a.TS"), "a").unwrap();
        fs::write(root.join("b.ts"), "b").unwrap();
        fs::write(root.join("c.md"), "c").unwrap();

        let results = walk_files(root, &[".ts"], &[]);
        assert_eq!(rel_sorted(root, &results), ["a.TS", "b.ts"]);
    }

    // --- load_ignore_for_dir / is_ignored ---

    #[test]
    fn load_returns_none_without_ignore_files() {
        let dir = tempdir().unwrap();
        assert!(load_ignore_for_dir(dir.path()).is_none());
    }

    #[test]
    fn load_combines_gitignore_and_cspignore() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "a.ts\n").unwrap();
        fs::write(dir.path().join(".cspignore"), "b.ts\n").unwrap();
        let spec = load_ignore_for_dir(dir.path()).unwrap();
        let pats: Vec<&str> = spec.patterns.iter().map(|p| p.pattern.as_str()).collect();
        assert_eq!(pats, ["a.ts", "b.ts"]);
    }

    #[test]
    fn load_skips_blanks_and_comments() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "# comment\n\n*.log\n").unwrap();
        let spec = load_ignore_for_dir(dir.path()).unwrap();
        assert_eq!(spec.patterns.len(), 1);
        assert_eq!(spec.patterns[0].pattern, "*.log");
    }

    #[test]
    fn is_ignored_found_for_negation_with_extension() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "*.log\n!special.log\n").unwrap();
        let spec = load_ignore_for_dir(dir.path()).unwrap();
        let check = is_ignored(&dir.path().join("special.log"), false, &[&spec]);
        assert!(!check.ignored);
        assert!(check.found);
        assert!(spec.has_negated_ext_pattern);
    }

    #[test]
    fn is_ignored_no_found_for_negation_without_extension() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "vendor/\n!vendor/keep/\n").unwrap();
        let spec = load_ignore_for_dir(dir.path()).unwrap();
        let check = is_ignored(&dir.path().join("vendor/keep"), true, &[&spec]);
        assert!(!check.found);
        assert!(!spec.has_negated_ext_pattern);
    }

    #[test]
    fn is_ignored_true_when_pattern_matches() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "*.log\n").unwrap();
        let spec = load_ignore_for_dir(dir.path()).unwrap();
        let check = is_ignored(&dir.path().join("foo.log"), false, &[&spec]);
        assert!(check.ignored);
    }

    #[test]
    fn has_negated_ext_pattern_false_without_negations() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join(".gitignore"), "*.log\n*.tmp\n").unwrap();
        let spec = load_ignore_for_dir(dir.path()).unwrap();
        assert!(!spec.has_negated_ext_pattern);
    }

    #[test]
    fn preserves_outer_ignored_state_across_specs() {
        let outer = tempdir().unwrap();
        fs::write(outer.path().join(".gitignore"), "*.log\n").unwrap();
        let outer_spec = load_ignore_for_dir(outer.path()).unwrap();

        let sub = outer.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join(".gitignore"), "*.tmp\n").unwrap();
        let inner_spec = load_ignore_for_dir(&sub).unwrap();

        let check = is_ignored(&sub.join("foo.log"), false, &[&outer_spec, &inner_spec]);
        assert!(check.ignored);
    }
}
