//! Path penalties and top-k reranking. Port of `src/ranking/penalties.ts`
//! (← semble `ranking/penalties.py`).
//!
//! Patterns operate on file paths only (no newlines), so the default
//! Unicode-aware regex matches the upstream JavaScript behavior for any
//! realistic (ASCII) path. (Unicode cannot be disabled here because the negated
//! class `[^/]` would then permit invalid-UTF-8 matches, which a string `Regex`
//! rejects.)

use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;

use crate::types::Chunk;

pub const STRONG_PENALTY: f64 = 0.3;
pub const MODERATE_PENALTY: f64 = 0.5;
pub const MILD_PENALTY: f64 = 0.7;

/// Maximum chunks from the same file before a saturation penalty applies.
pub const FILE_SATURATION_THRESHOLD: usize = 1;
/// Multiplicative penalty per extra chunk from the same file beyond the
/// threshold.
pub const FILE_SATURATION_DECAY: f64 = 0.5;

/// Filenames that are re-export barrels or package-level metadata.
const REEXPORT_FILENAMES: [&str; 2] = ["__init__.py", "package-info.java"];

fn compile(pattern: &str) -> Regex {
    Regex::new(pattern).expect("penalty regex is valid")
}

/// Test files across common languages (see the upstream `TEST_FILE_RE`).
static TEST_FILE_RE: LazyLock<Regex> = LazyLock::new(|| {
    compile(concat!(
        r"(?:^|/)(?:",
        r"test_[^/]*\.py|[^/]*_test\.py",
        r"|[^/]*_test\.go",
        r"|[^/]*Tests?\.java",
        r"|[^/]*Test\.php",
        r"|[^/]*_spec\.rb|[^/]*_test\.rb",
        r"|[^/]*\.test\.[jt]sx?|[^/]*\.spec\.[jt]sx?",
        r"|[^/]*Tests?\.kt|[^/]*Spec\.kt",
        r"|[^/]*Tests?\.swift|[^/]*Spec\.swift",
        r"|[^/]*Tests?\.cs",
        r"|test_[^/]*\.cpp|[^/]*_test\.cpp|test_[^/]*\.c|[^/]*_test\.c",
        r"|[^/]*Spec\.scala|[^/]*Suite\.scala|[^/]*Test\.scala",
        r"|[^/]*_test\.dart|test_[^/]*\.dart",
        r"|[^/]*_spec\.lua|[^/]*_test\.lua|test_[^/]*\.lua",
        r"|test_helper[^/]*\.\w+",
        r")$",
    ))
});

/// Test/spec directories.
static TEST_DIR_RE: LazyLock<Regex> =
    LazyLock::new(|| compile(r"(?:^|/)(?:tests?|__tests__|spec|testing)(?:/|$)"));
/// Compat/legacy path components.
static COMPAT_DIR_RE: LazyLock<Regex> =
    LazyLock::new(|| compile(r"(?:^|/)(?:compat|_compat|legacy)(?:/|$)"));
/// Examples/docs path components.
static EXAMPLES_DIR_RE: LazyLock<Regex> =
    LazyLock::new(|| compile(r"(?:^|/)(?:_?examples?|docs?_src)(?:/|$)"));
/// TypeScript declaration files.
static TYPE_DEFS_RE: LazyLock<Regex> = LazyLock::new(|| compile(r"\.d\.ts$"));

/// Return a combined multiplicative penalty for all applicable path patterns.
pub fn file_path_penalty(file_path: &str) -> f64 {
    let normalised = file_path.replace('\\', "/");
    let mut penalty = 1.0;

    if TEST_FILE_RE.is_match(&normalised) || TEST_DIR_RE.is_match(&normalised) {
        penalty *= STRONG_PENALTY;
    }
    // Match Python's `Path(file_path).name` (POSIX): only `/` is a separator,
    // so backslashes in the raw path are part of the filename.
    let basename = match file_path.rfind('/') {
        Some(i) => &file_path[i + 1..],
        None => file_path,
    };
    if REEXPORT_FILENAMES.contains(&basename) {
        penalty *= MODERATE_PENALTY;
    }
    if COMPAT_DIR_RE.is_match(&normalised) {
        penalty *= STRONG_PENALTY;
    }
    if EXAMPLES_DIR_RE.is_match(&normalised) {
        penalty *= STRONG_PENALTY;
    }
    if TYPE_DEFS_RE.is_match(&normalised) {
        penalty *= MILD_PENALTY;
    }
    penalty
}

/// Descending comparison for scores, treating incomparable (`NaN`) as equal so
/// the sort stays stable (mirrors JS `(a, b) => b - a` over finite scores).
fn by_score_desc(a: f64, b: f64) -> Ordering {
    b.partial_cmp(&a).unwrap_or(Ordering::Equal)
}

/// Select top-k results with optional file-path penalties and file-saturation
/// decay. Scores are keyed by chunk index into `chunks`; results are returned as
/// `(chunk_index, final_score)` pairs, highest first.
pub fn rerank_top_k(
    scores: &super::Scores,
    chunks: &[Chunk],
    top_k: usize,
    penalise_paths: bool,
) -> Vec<(usize, f64)> {
    if scores.is_empty() || top_k == 0 {
        return Vec::new();
    }

    // Apply file-path penalties (cached per path), preserving insertion order.
    let mut penalty_cache: HashMap<&str, f64> = HashMap::new();
    let mut penalised: IndexMap<usize, f64> = IndexMap::with_capacity(scores.len());
    for (&idx, &score) in scores {
        let file_path = chunks[idx].file_path.as_str();
        let pen = if penalise_paths {
            *penalty_cache
                .entry(file_path)
                .or_insert_with(|| file_path_penalty(file_path))
        } else {
            1.0
        };
        penalised.insert(idx, score * pen);
    }

    // Sort indices by penalised score (highest first); stable → ties keep
    // insertion order, matching the upstream single stable sort.
    let mut ranked: Vec<usize> = penalised.keys().copied().collect();
    ranked.sort_by(|&a, &b| by_score_desc(penalised[&a], penalised[&b]));

    let mut file_selected: HashMap<&str, usize> = HashMap::new();
    let mut selected: Vec<(f64, usize)> = Vec::new();
    let mut min_selected = f64::INFINITY;

    for &idx in &ranked {
        let pen_score = penalised[&idx];
        if selected.len() >= top_k && pen_score <= min_selected {
            break;
        }

        let file_path = chunks[idx].file_path.as_str();
        let already = file_selected.get(file_path).copied().unwrap_or(0);
        let mut eff_score = pen_score;
        if already >= FILE_SATURATION_THRESHOLD {
            let excess = already - FILE_SATURATION_THRESHOLD + 1;
            eff_score *= FILE_SATURATION_DECAY.powi(excess as i32);
        }

        selected.push((eff_score, idx));
        file_selected.insert(file_path, already + 1);

        if selected.len() >= top_k {
            min_selected = selected
                .iter()
                .map(|&(s, _)| s)
                .fold(f64::INFINITY, f64::min);
        }
    }

    selected.sort_by(|a, b| by_score_desc(a.0, b.0));
    selected.truncate(top_k);
    selected
        .into_iter()
        .map(|(score, idx)| (idx, score))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunk(file_path: &str, idx: u32) -> Chunk {
        Chunk {
            content: format!("chunk {idx}"),
            file_path: file_path.to_string(),
            start_line: idx,
            end_line: idx + 1,
            language: None,
        }
    }

    fn scores_from(pairs: &[(usize, f64)]) -> super::super::Scores {
        pairs.iter().copied().collect()
    }

    // --- _filePathPenalty (mirrors src/ranking/penalties.test.ts) ---

    #[test]
    fn penalises_js_ts_test_files() {
        assert_eq!(file_path_penalty("src/foo.test.ts"), STRONG_PENALTY);
        assert_eq!(file_path_penalty("src/foo.spec.tsx"), STRONG_PENALTY);
    }

    #[test]
    fn penalises_reexport_barrel() {
        assert_eq!(file_path_penalty("src/__init__.py"), MODERATE_PENALTY);
        assert_eq!(file_path_penalty("__init__.py"), MODERATE_PENALTY);
    }

    #[test]
    fn penalises_type_stubs() {
        assert_eq!(file_path_penalty("src/foo.d.ts"), MILD_PENALTY);
        // Only `.d.ts` matches; basename is `__init__.d.ts`, not a barrel.
        assert_eq!(file_path_penalty("src/__init__.d.ts"), MILD_PENALTY);
    }

    #[test]
    fn test_dir_and_test_file_share_one_strong_branch() {
        assert!((file_path_penalty("tests/test_foo.py") - STRONG_PENALTY).abs() < 1e-10);
    }

    #[test]
    fn ordinary_files_are_unpenalised() {
        assert_eq!(file_path_penalty("src/foo.ts"), 1.0);
    }

    #[test]
    fn compounds_strong_penalties() {
        assert!(
            (file_path_penalty("examples/foo.test.ts") - STRONG_PENALTY * STRONG_PENALTY).abs()
                < 1e-10
        );
    }

    #[test]
    fn penalises_dirs_and_other_languages() {
        assert_eq!(file_path_penalty("compat/foo.ts"), STRONG_PENALTY);
        assert_eq!(file_path_penalty("examples/foo.ts"), STRONG_PENALTY);
        assert_eq!(file_path_penalty("legacy/foo.ts"), STRONG_PENALTY);
        assert_eq!(file_path_penalty("pkg/foo_test.go"), STRONG_PENALTY);
        assert_eq!(file_path_penalty("src/FooTests.java"), STRONG_PENALTY);
    }

    #[test]
    fn normalises_backslashes_before_matching() {
        assert_eq!(file_path_penalty("src\\foo.test.ts"), STRONG_PENALTY);
    }

    // --- rerankTopK ---

    #[test]
    fn empty_input_returns_empty() {
        let chunks: Vec<Chunk> = vec![];
        assert!(rerank_top_k(&scores_from(&[]), &chunks, 5, true).is_empty());
    }

    #[test]
    fn non_positive_topk_returns_empty() {
        let chunks = [chunk("a.ts", 0)];
        let scores = scores_from(&[(0, 1.0)]);
        assert!(rerank_top_k(&scores, &chunks, 0, true).is_empty());
    }

    #[test]
    fn applies_saturation_decay_within_a_file() {
        let chunks = [
            chunk("src/foo.ts", 0),
            chunk("src/foo.ts", 1),
            chunk("src/foo.ts", 2),
            chunk("src/foo.ts", 3),
        ];
        let scores = scores_from(&[(0, 1.0), (1, 1.0), (2, 1.0), (3, 1.0)]);
        let result = rerank_top_k(&scores, &chunks, 4, false);
        assert_eq!(result.len(), 4);
        let s: Vec<f64> = result.iter().map(|&(_, s)| s).collect();
        assert!((s[0] - 1.0).abs() < 1e-10);
        assert!((s[1] - FILE_SATURATION_DECAY).abs() < 1e-10);
        assert!((s[2] - FILE_SATURATION_DECAY.powi(2)).abs() < 1e-10);
        assert!((s[3] - FILE_SATURATION_DECAY.powi(3)).abs() < 1e-10);
    }

    #[test]
    fn truncates_to_topk_after_sorting() {
        let chunks = [chunk("a.ts", 0), chunk("b.ts", 1), chunk("c.ts", 2)];
        let scores = scores_from(&[(0, 0.5), (1, 0.9), (2, 0.1)]);
        let result = rerank_top_k(&scores, &chunks, 2, false);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].0, 1); // b
        assert_eq!(result[1].0, 0); // a
    }

    #[test]
    fn applies_path_penalties_before_sorting() {
        let chunks = [chunk("src/foo.test.ts", 0), chunk("src/foo.ts", 1)];
        let scores = scores_from(&[(0, 0.9), (1, 0.5)]);
        let result = rerank_top_k(&scores, &chunks, 2, true);
        assert_eq!(result[0].0, 1); // b wins post-penalty
        assert_eq!(result[1].0, 0);
        assert!((result[0].1 - 0.5).abs() < 1e-10);
        assert!((result[1].1 - 0.9 * STRONG_PENALTY).abs() < 1e-10);
    }

    #[test]
    fn skips_path_penalties_when_disabled() {
        let chunks = [chunk("src/foo.test.ts", 0), chunk("src/foo.ts", 1)];
        let scores = scores_from(&[(0, 0.9), (1, 0.5)]);
        let result = rerank_top_k(&scores, &chunks, 2, false);
        assert_eq!(result[0].0, 0);
        assert!((result[0].1 - 0.9).abs() < 1e-10);
        assert_eq!(result[1].0, 1);
        assert!((result[1].1 - 0.5).abs() < 1e-10);
    }

    #[test]
    fn mixes_saturation_decay_across_files() {
        let chunks = [
            chunk("a.ts", 0),
            chunk("a.ts", 1),
            chunk("b.ts", 2),
            chunk("b.ts", 3),
        ];
        let scores = scores_from(&[(0, 1.0), (1, 1.0), (2, 1.0), (3, 1.0)]);
        let result = rerank_top_k(&scores, &chunks, 4, false);
        assert_eq!(result.len(), 4);
        let s: Vec<f64> = result.iter().map(|&(_, sc)| sc).collect();
        assert!((s[0] - 1.0).abs() < 1e-10);
        assert!((s[1] - 1.0).abs() < 1e-10);
        assert!((s[2] - FILE_SATURATION_DECAY).abs() < 1e-10);
        assert!((s[3] - FILE_SATURATION_DECAY).abs() < 1e-10);
    }
}
