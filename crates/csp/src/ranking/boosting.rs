//! Query-type boosting. Port of `src/ranking/boosting.ts` (← semble
//! `ranking/boosting.py`).
//!
//! Definition detection uses `fancy-regex` because the upstream patterns rely
//! on a lookbehind (`(?<=\s)`) that the `regex` crate does not support; the
//! patterns are otherwise transcribed verbatim. Other patterns
//! (`SYMBOL_QUERY_RE`, `EMBEDDED_SYMBOL_RE`, `QUERY_WORD_RE`) use the `regex`
//! crate. Score maps are [`super::Scores`] (`IndexMap<usize, f64>`), the Rust
//! analogue of the TS `Map<Chunk, number>` keyed by object identity.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::LazyLock;

use fancy_regex::Regex as FancyRegex;
use regex::{Regex, RegexBuilder};

use super::Scores;
use crate::tokens::split_identifier;
use crate::types::Chunk;

// --- constants (mirroring the upstream module) -----------------------------

const EMBEDDED_STEM_MIN_LEN: usize = 4;
const EMBEDDED_SYMBOL_BOOST_SCALE: f64 = 0.5;
const DEFINITION_BOOST_MULTIPLIER: f64 = 3.0;
const STEM_BOOST_MULTIPLIER: f64 = 1.0;
const FILE_COHERENCE_BOOST_FRAC: f64 = 0.2;

// Case-sensitive general definition keywords.
const DEFINITION_KEYWORDS: [&str; 21] = [
    "class",
    "module",
    "defmodule",
    "def",
    "interface",
    "struct",
    "enum",
    "trait",
    "type",
    "func",
    "function",
    "object",
    "abstract class",
    "data class",
    "fn",
    "fun",
    "package",
    "namespace",
    "protocol",
    "record",
    "typedef",
];

// SQL DDL keywords (matched case-insensitively).
const SQL_DEFINITION_KEYWORDS: [&str; 4] = [
    "CREATE TABLE",
    "CREATE VIEW",
    "CREATE PROCEDURE",
    "CREATE FUNCTION",
];

static STOPWORDS: LazyLock<HashSet<&'static str>> = LazyLock::new(|| {
    "a an and are as at be by do does for from has have how if in is it not of on or the to was \
     what when where which who why with"
        .split(' ')
        .collect()
});

// --- regexes ---------------------------------------------------------------

/// Symbol-lookup queries: namespace-qualified, leading-underscore, or
/// containing uppercase/underscore (`\w`/`\d` written as explicit ASCII classes,
/// Unicode disabled, to match JavaScript semantics).
static SYMBOL_QUERY_RE: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(
        r"^(?:[A-Z_a-z][A-Za-z0-9_]*(?:(?:::|\\|->|\.)[A-Z_a-z][A-Za-z0-9_]*)+|_[A-Za-z0-9_]*|[A-Za-z][0-9a-z]*[A-Z_][A-Za-z0-9_]*|[A-Z][A-Za-z0-9]*)$",
    )
    .unicode(false)
    .build()
    .expect("SYMBOL_QUERY_RE is a valid regex")
});

/// CamelCase/camelCase identifiers embedded in an NL query; excludes plain
/// words and pure acronyms.
static EMBEDDED_SYMBOL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:[A-Z][a-z][0-9a-z]*[A-Z][0-9A-Za-z]*|[a-z][0-9a-z]*[A-Z][0-9A-Za-z]+)\b")
        .expect("EMBEDDED_SYMBOL_RE is a valid regex")
});

/// Query words for stem matching (`/[A-Z_]\w*/gi`).
static QUERY_WORD_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").expect("QUERY_WORD_RE is a valid regex")
});

/// Return true if the query looks like a bare symbol or namespace-qualified
/// identifier.
pub fn is_symbol_query(query: &str) -> bool {
    SYMBOL_QUERY_RE.is_match(query.trim())
}

// --- definition patterns (fancy-regex; cached per symbol name) -------------

fn escape_regex(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(
            c,
            '.' | '*' | '+' | '?' | '^' | '$' | '{' | '}' | '(' | ')' | '|' | '[' | ']' | '\\'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

static DEFINITION_KEYWORD_BODY: LazyLock<String> = LazyLock::new(|| {
    DEFINITION_KEYWORDS
        .iter()
        .map(|k| escape_regex(k))
        .collect::<Vec<_>>()
        .join("|")
});
static SQL_KEYWORD_BODY: LazyLock<String> = LazyLock::new(|| {
    SQL_DEFINITION_KEYWORDS
        .iter()
        .map(|k| escape_regex(k))
        .collect::<Vec<_>>()
        .join("|")
});

const NS_PREFIX: &str = r"(?:[A-Z_a-z]\w*(?:\.|::))*";
const DEF_SUFFIX_TAIL: &str = r"(?:\s|[<({:\[;]|$)";

fn build_definition_pattern(flags: &str, keyword_body: &str, escaped: &str) -> FancyRegex {
    // flags + `(?:^|(?<=\s))(?:<keywords>)\s+<ns-prefix><name><tail>`
    let mut pattern = String::new();
    pattern.push_str(flags);
    pattern.push_str(r"(?:^|(?<=\s))(?:");
    pattern.push_str(keyword_body);
    pattern.push_str(r")\s+");
    pattern.push_str(NS_PREFIX);
    pattern.push_str(escaped);
    pattern.push_str(DEF_SUFFIX_TAIL);
    FancyRegex::new(&pattern).expect("definition pattern is valid")
}

type DefPatterns = (FancyRegex, FancyRegex);

thread_local! {
    static DEFINITION_PATTERN_CACHE: RefCell<HashMap<String, Rc<DefPatterns>>> =
        RefCell::new(HashMap::new());
}

fn definition_pattern(symbol_name: &str) -> Rc<DefPatterns> {
    DEFINITION_PATTERN_CACHE.with(|cache| {
        if let Some(found) = cache.borrow().get(symbol_name) {
            return Rc::clone(found);
        }
        let escaped = escape_regex(symbol_name);
        let general = build_definition_pattern("(?m)", &DEFINITION_KEYWORD_BODY, &escaped);
        let sql = build_definition_pattern("(?im)", &SQL_KEYWORD_BODY, &escaped);
        let entry = Rc::new((general, sql));
        cache
            .borrow_mut()
            .insert(symbol_name.to_string(), Rc::clone(&entry));
        entry
    })
}

/// Return true if the chunk contains a definition of `symbol_name`.
/// Case-sensitive for general keywords, case-insensitive for SQL DDL.
pub fn chunk_defines_symbol(chunk: &Chunk, symbol_name: &str) -> bool {
    let patterns = definition_pattern(symbol_name);
    patterns.0.is_match(&chunk.content).unwrap_or(false)
        || patterns.1.is_match(&chunk.content).unwrap_or(false)
}

// --- path helpers ----------------------------------------------------------

/// Python `Path.stem` (original case): filename without its final suffix,
/// leaving a leading-dot file untouched.
fn path_stem_original(file_path: &str) -> &str {
    let base = match file_path.rfind(['/', '\\']) {
        Some(i) => &file_path[i + 1..],
        None => file_path,
    };
    match base.rfind('.') {
        Some(0) | None => base,
        Some(i) => &base[..i],
    }
}

fn path_stem_lower(file_path: &str) -> String {
    path_stem_original(file_path).to_lowercase()
}

fn path_parent_name(file_path: &str) -> String {
    let cleaned = file_path.trim_end_matches(['/', '\\']);
    let Some(sep) = cleaned.rfind(['/', '\\']) else {
        return String::new();
    };
    let parent = &cleaned[..sep];
    match parent.rfind(['/', '\\']) {
        Some(j) => parent[j + 1..].to_string(),
        None => parent.to_string(),
    }
}

// --- stem matching ---------------------------------------------------------

fn strip_trailing_s(s: &str) -> &str {
    s.trim_end_matches('s')
}

/// True if `stem` matches `name` (exact, snake-stripped, or plural).
pub fn stem_matches(stem: &str, name: &str) -> bool {
    let stem_norm = stem.replace('_', "");
    stem == name
        || stem_norm == name
        || strip_trailing_s(stem) == name
        || strip_trailing_s(&stem_norm) == name
}

/// Extract the final identifier from a possibly namespace-qualified query.
pub fn extract_symbol_name(query: &str) -> String {
    for separator in ["::", "\\", "->", "."] {
        if let Some(idx) = query.rfind(separator) {
            return query[idx + separator.len()..].to_string();
        }
    }
    query.trim().to_string()
}

// --- scoring helpers -------------------------------------------------------

fn max_value(scores: &Scores) -> f64 {
    scores.values().copied().fold(f64::NEG_INFINITY, f64::max)
}

/// Boost amount for a chunk that defines one of `names` (0.0 if none match);
/// 1.5× when the file stem also matches a name, else 1.0×.
fn definition_tier(chunk: &Chunk, names: &[String], boost_unit: f64) -> f64 {
    if !names.iter().any(|n| chunk_defines_symbol(chunk, n)) {
        return 0.0;
    }
    let stem = path_stem_lower(&chunk.file_path);
    for name in names {
        if stem_matches(&stem, &name.to_lowercase()) {
            return boost_unit * 1.5;
        }
    }
    boost_unit
}

fn scan_non_candidates(
    boosted: &mut Scores,
    names: &[String],
    boost_unit: f64,
    chunks: &[Chunk],
    stem_ok: impl Fn(&str) -> bool,
) {
    for (idx, chunk) in chunks.iter().enumerate() {
        if boosted.contains_key(&idx) {
            continue;
        }
        if !stem_ok(&path_stem_lower(&chunk.file_path)) {
            continue;
        }
        let tier = definition_tier(chunk, names, boost_unit);
        if tier != 0.0 {
            boosted.insert(idx, tier);
        }
    }
}

fn boost_symbol_definitions(boosted: &mut Scores, query: &str, max_score: f64, chunks: &[Chunk]) {
    let symbol_name = extract_symbol_name(query);
    let trimmed = query.trim().to_string();
    let mut names: Vec<String> = vec![symbol_name.clone()];
    if symbol_name != trimmed {
        names.push(trimmed);
    }

    let boost_unit = max_score * DEFINITION_BOOST_MULTIPLIER;

    let keys: Vec<usize> = boosted.keys().copied().collect();
    for idx in keys {
        let tier = definition_tier(&chunks[idx], &names, boost_unit);
        if tier != 0.0 {
            let current = boosted[&idx];
            boosted.insert(idx, current + tier);
        }
    }

    let symbol_lower = symbol_name.to_lowercase();
    scan_non_candidates(boosted, &names, boost_unit, chunks, |stem| {
        stem_matches(stem, &symbol_lower)
    });
}

fn dedup_preserving_order(values: Vec<String>) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out = Vec::new();
    for v in values {
        if seen.insert(v.clone()) {
            out.push(v);
        }
    }
    out
}

fn boost_embedded_symbols(boosted: &mut Scores, query: &str, max_score: f64, chunks: &[Chunk]) {
    let names = dedup_preserving_order(
        EMBEDDED_SYMBOL_RE
            .find_iter(query)
            .map(|m| m.as_str().to_string())
            .collect(),
    );
    if names.is_empty() {
        return;
    }

    let boost_unit = max_score * DEFINITION_BOOST_MULTIPLIER * EMBEDDED_SYMBOL_BOOST_SCALE;

    let keys: Vec<usize> = boosted.keys().copied().collect();
    for idx in keys {
        let tier = definition_tier(&chunks[idx], &names, boost_unit);
        if tier != 0.0 {
            let current = boosted[&idx];
            boosted.insert(idx, current + tier);
        }
    }

    let symbols_lower: Vec<String> = names.iter().map(|s| s.to_lowercase()).collect();
    for (idx, chunk) in chunks.iter().enumerate() {
        if boosted.contains_key(&idx) {
            continue;
        }
        let stem = path_stem_lower(&chunk.file_path);
        let stem_norm = stem.replace('_', "");
        let matches = symbols_lower.iter().any(|sl| {
            stem == *sl
                || stem_norm == *sl
                || (stem.len() >= EMBEDDED_STEM_MIN_LEN && sl.starts_with(stem.as_str()))
                || (stem_norm.len() >= EMBEDDED_STEM_MIN_LEN && sl.starts_with(stem_norm.as_str()))
        });
        if !matches {
            continue;
        }
        let tier = definition_tier(chunk, &names, boost_unit);
        if tier != 0.0 {
            boosted.insert(idx, tier);
        }
    }
}

/// Count query keywords matching path parts, allowing prefix overlap (min 3
/// chars).
pub fn count_keyword_matches(keywords: &HashSet<String>, parts: &HashSet<String>) -> usize {
    let mut exact: HashSet<&String> = HashSet::new();
    let mut exact_count = 0;
    for k in keywords {
        if parts.contains(k) {
            exact.insert(k);
            exact_count += 1;
        }
    }
    if exact_count == keywords.len() {
        return exact_count;
    }
    let mut n_matches = exact_count;
    for keyword in keywords {
        if exact.contains(keyword) {
            continue;
        }
        for part in parts {
            let (shorter, longer) = if keyword.len() <= part.len() {
                (keyword, part)
            } else {
                (part, keyword)
            };
            if shorter.len() >= 3 && longer.starts_with(shorter.as_str()) {
                n_matches += 1;
                break;
            }
        }
    }
    n_matches
}

fn boost_stem_matches(boosted: &mut Scores, query: &str, max_score: f64, chunks: &[Chunk]) {
    let mut keywords: HashSet<String> = HashSet::new();
    for m in QUERY_WORD_RE.find_iter(query) {
        let word = m.as_str();
        if word.len() > 2 {
            let lower = word.to_lowercase();
            if !STOPWORDS.contains(lower.as_str()) {
                keywords.insert(lower);
            }
        }
    }
    if keywords.is_empty() {
        return;
    }

    let boost = max_score * STEM_BOOST_MULTIPLIER;
    let mut path_cache: HashMap<String, HashSet<String>> = HashMap::new();
    let keys: Vec<usize> = boosted.keys().copied().collect();
    for idx in keys {
        let file_path = chunks[idx].file_path.clone();
        let parts = path_cache.entry(file_path).or_insert_with_key(|fp| {
            let mut parts: HashSet<String> = split_identifier(path_stem_original(fp))
                .into_iter()
                .collect();
            let parent = path_parent_name(fp);
            if !parent.is_empty() && parent != "." && parent != "/" && parent != ".." {
                for p in split_identifier(&parent) {
                    parts.insert(p);
                }
            }
            parts
        });
        let n_matches = count_keyword_matches(&keywords, parts);
        if n_matches > 0 {
            let match_ratio = n_matches as f64 / keywords.len() as f64;
            if match_ratio >= 0.10 {
                let current = boosted[&idx];
                boosted.insert(idx, current + boost * match_ratio);
            }
        }
    }
}

// --- public API ------------------------------------------------------------

/// Apply query-type boosts to candidate scores, returning a new map.
pub fn apply_query_boost(combined: &Scores, query: &str, chunks: &[Chunk]) -> Scores {
    if combined.is_empty() {
        return Scores::new();
    }
    let max_score = max_value(combined);
    let mut boosted = combined.clone();

    if is_symbol_query(query) {
        boost_symbol_definitions(&mut boosted, query, max_score, chunks);
    } else {
        boost_stem_matches(&mut boosted, query, max_score, chunks);
        boost_embedded_symbols(&mut boosted, query, max_score, chunks);
    }

    boosted
}

/// Promote files with multiple high-scoring chunks by boosting their top chunk
/// (in place).
pub fn boost_multi_chunk_files(scores: &mut Scores, chunks: &[Chunk]) {
    if scores.is_empty() {
        return;
    }
    let max_score = max_value(scores);
    if max_score == 0.0 {
        return;
    }

    let mut file_sum: HashMap<String, f64> = HashMap::new();
    let mut best_chunk: HashMap<String, usize> = HashMap::new();
    for (&idx, &score) in scores.iter() {
        let file_path = chunks[idx].file_path.clone();
        *file_sum.entry(file_path.clone()).or_insert(0.0) += score;
        match best_chunk.get(&file_path) {
            None => {
                best_chunk.insert(file_path, idx);
            }
            Some(&existing) if score > scores[&existing] => {
                best_chunk.insert(file_path, idx);
            }
            Some(_) => {}
        }
    }

    let max_file_sum = file_sum.values().copied().fold(f64::NEG_INFINITY, f64::max);
    // Guard against zero/negative max to avoid NaN/Infinity from the division.
    if max_file_sum <= 0.0 {
        return;
    }
    let boost_unit = max_score * FILE_COHERENCE_BOOST_FRAC;
    for (file_path, &idx) in &best_chunk {
        let sum = file_sum[file_path];
        let current = scores[&idx];
        scores.insert(idx, current + boost_unit * sum / max_file_sum);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_chunk(content: &str, file_path: &str) -> Chunk {
        Chunk {
            content: content.to_string(),
            file_path: file_path.to_string(),
            start_line: 1,
            end_line: 10,
            language: None,
        }
    }

    fn scores_of(pairs: &[(usize, f64)]) -> Scores {
        pairs.iter().copied().collect()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-10
    }

    // --- isSymbolQuery ---

    #[test]
    fn symbol_query_classification() {
        assert!(is_symbol_query("HandlerStack"));
        assert!(is_symbol_query("Client"));
        assert!(is_symbol_query("Sinatra::Base"));
        assert!(is_symbol_query("Phoenix.Router"));
        assert!(is_symbol_query("foo->bar"));
        assert!(is_symbol_query(r"A\B\C"));
        assert!(is_symbol_query("_private"));
        assert!(is_symbol_query("_"));
        assert!(is_symbol_query("my_func"));
        assert!(!is_symbol_query("session"));
        assert!(!is_symbol_query("foo"));
        assert!(!is_symbol_query("how does this work"));
        assert!(is_symbol_query("  HandlerStack  "));
    }

    // --- extract_symbol_name ---

    #[test]
    fn extracts_symbol_name() {
        assert_eq!(extract_symbol_name("Sinatra::Base"), "Base");
        assert_eq!(extract_symbol_name("Phoenix.Router"), "Router");
        assert_eq!(extract_symbol_name("foo->bar"), "bar");
        assert_eq!(extract_symbol_name("Client"), "Client");
        assert_eq!(extract_symbol_name("  Client  "), "Client");
    }

    // --- stem_matches ---

    #[test]
    fn stem_matching() {
        assert!(stem_matches("client", "client"));
        assert!(stem_matches("handler_stack", "handlerstack"));
        assert!(stem_matches("clients", "client"));
        assert!(stem_matches("handler_stacks", "handlerstack"));
        assert!(!stem_matches("foo", "bar"));
    }

    // --- chunk_defines_symbol ---

    #[test]
    fn defines_class() {
        assert!(chunk_defines_symbol(
            &mk_chunk("class HandlerStack:\n    pass\n", "a.py"),
            "HandlerStack"
        ));
    }

    #[test]
    fn defines_function() {
        assert!(chunk_defines_symbol(
            &mk_chunk("def my_func(x):\n    return x\n", "a.py"),
            "my_func"
        ));
    }

    #[test]
    fn defines_namespace_qualified_for_trailing_name() {
        assert!(chunk_defines_symbol(
            &mk_chunk("defmodule Phoenix.Router do\nend\n", "a.ex"),
            "Router"
        ));
    }

    #[test]
    fn case_sensitive_does_not_match_module_keyword() {
        assert!(!chunk_defines_symbol(
            &mk_chunk("Module Foo", "a.txt"),
            "Foo"
        ));
    }

    #[test]
    fn case_insensitive_for_sql_ddl() {
        assert!(chunk_defines_symbol(
            &mk_chunk("create table users (id int);", "a.sql"),
            "users"
        ));
        assert!(chunk_defines_symbol(
            &mk_chunk("CREATE TABLE users (id int);", "a.sql"),
            "users"
        ));
    }

    #[test]
    fn does_not_match_mid_word() {
        assert!(!chunk_defines_symbol(
            &mk_chunk("# subclass Foo\n", "a.py"),
            "Foo"
        ));
    }

    // --- count_keyword_matches ---

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn counts_exact_and_prefix_matches() {
        assert_eq!(
            count_keyword_matches(&set(&["foo", "bar"]), &set(&["foo", "bar", "baz"])),
            2
        );
        assert_eq!(
            count_keyword_matches(&set(&["dep"]), &set(&["dependency"])),
            1
        );
        assert_eq!(
            count_keyword_matches(&set(&["depend"]), &set(&["dependencies"])),
            1
        );
        assert_eq!(
            count_keyword_matches(&set(&["dependency"]), &set(&["dep"])),
            1
        );
        assert_eq!(
            count_keyword_matches(&set(&["de"]), &set(&["dependency"])),
            0
        );
    }

    // --- boost_multi_chunk_files ---

    #[test]
    fn multi_chunk_boost_top_chunk() {
        let chunks = [
            mk_chunk("x", "a.ts"),
            mk_chunk("y", "a.ts"),
            mk_chunk("z", "a.ts"),
            mk_chunk("q", "b.ts"),
        ];
        let mut scores = scores_of(&[(0, 0.5), (1, 0.4), (2, 0.3), (3, 0.2)]);
        boost_multi_chunk_files(&mut scores, &chunks);
        assert!(close(scores[&0], 0.6));
        assert!(close(scores[&1], 0.4));
        assert!(close(scores[&2], 0.3));
        assert!(close(scores[&3], 0.2 + 0.1 * 0.2 / 1.2));
    }

    #[test]
    fn multi_chunk_noop_on_empty() {
        let chunks: Vec<Chunk> = vec![];
        let mut scores = Scores::new();
        boost_multi_chunk_files(&mut scores, &chunks);
        assert!(scores.is_empty());
    }

    #[test]
    fn multi_chunk_noop_when_max_zero() {
        let chunks = [mk_chunk("x", "a.ts")];
        let mut scores = scores_of(&[(0, 0.0)]);
        boost_multi_chunk_files(&mut scores, &chunks);
        assert_eq!(scores[&0], 0.0);
    }

    #[test]
    fn multi_chunk_no_nan_when_sums_cancel() {
        let chunks = [mk_chunk("x", "a.ts"), mk_chunk("y", "a.ts")];
        let mut scores = scores_of(&[(0, 1.0), (1, -1.0)]);
        boost_multi_chunk_files(&mut scores, &chunks);
        assert_eq!(scores[&0], 1.0);
        assert_eq!(scores[&1], -1.0);
    }

    #[test]
    fn multi_chunk_uses_coherence_frac() {
        let chunks = [mk_chunk("x", "a.ts")];
        let mut scores = scores_of(&[(0, 1.0)]);
        boost_multi_chunk_files(&mut scores, &chunks);
        assert!(close(scores[&0], 1.0 + FILE_COHERENCE_BOOST_FRAC));
    }

    // --- apply_query_boost ---

    #[test]
    fn symbol_boost_one_x_when_stem_mismatch() {
        let chunks = [
            mk_chunk("class HandlerStack:\n    pass\n", "other.py"),
            mk_chunk("print(\"hi\")", "b.py"),
        ];
        let scores = scores_of(&[(0, 0.5), (1, 1.0)]);
        let boosted = apply_query_boost(&scores, "HandlerStack", &chunks);
        assert!(close(boosted[&0], 0.5 + DEFINITION_BOOST_MULTIPLIER));
        assert_eq!(boosted[&1], 1.0);
    }

    #[test]
    fn symbol_boost_one_point_five_x_on_stem_match() {
        let chunks = [mk_chunk(
            "class HandlerStack:\n    pass\n",
            "handler_stack.py",
        )];
        let scores = scores_of(&[(0, 0.5)]);
        let boosted = apply_query_boost(&scores, "HandlerStack", &chunks);
        assert!(close(boosted[&0], 2.75));
    }

    #[test]
    fn symbol_boost_promotes_non_candidate() {
        let chunks = [
            mk_chunk("print(\"hi\")", "b.py"),
            mk_chunk("class HandlerStack:\n    pass\n", "handler_stack.py"),
        ];
        let scores = scores_of(&[(0, 1.0)]);
        let boosted = apply_query_boost(&scores, "HandlerStack", &chunks);
        assert!(close(boosted[&1], 4.5));
    }

    #[test]
    fn nl_embedded_pascal_case_half_strength() {
        let chunks = [mk_chunk(
            "class StateManager:\n    pass\n",
            "state_manager.py",
        )];
        let scores = scores_of(&[(0, 1.0)]);
        let boosted = apply_query_boost(
            &scores,
            "where does the StateManager initialize state",
            &chunks,
        );
        let expected = DEFINITION_BOOST_MULTIPLIER * EMBEDDED_SYMBOL_BOOST_SCALE * 1.5;
        assert!(boosted[&0] >= 1.0 + expected - 1e-9);
    }

    #[test]
    fn returns_new_map_without_mutating_input() {
        let chunks = [mk_chunk("class Foo:\n    pass\n", "foo.py")];
        let original = scores_of(&[(0, 1.0)]);
        let boosted = apply_query_boost(&original, "Foo", &chunks);
        assert_eq!(original[&0], 1.0);
        assert!(boosted[&0] > 1.0);
    }

    #[test]
    fn empty_input_returns_fresh_map() {
        let chunks: Vec<Chunk> = vec![];
        let out = apply_query_boost(&Scores::new(), "foo", &chunks);
        assert!(out.is_empty());
    }

    #[test]
    fn nl_stem_match_boost() {
        let chunks = [mk_chunk("print(\"hi\")", "cache_layer.py")];
        let scores = scores_of(&[(0, 1.0)]);
        let boosted = apply_query_boost(&scores, "find the cache layer", &chunks);
        assert!(close(boosted[&0], 1.0 + 2.0 / 3.0));
    }
}
