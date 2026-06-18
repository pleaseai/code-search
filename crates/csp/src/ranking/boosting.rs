//! Query-type boosting. Port of `src/ranking/boosting.ts` (← semble
//! `ranking/boosting.py`).
//!
//! Phase 1 status: `is_symbol_query` (consumed by [`super::weighting`]) is
//! ported. The remaining boost logic (`apply_query_boost`,
//! `boost_multi_chunk_files`, definition-pattern detection, embedded-symbol and
//! stem boosting) lands with task T006 — it relies on `fancy-regex` for the
//! upstream definition patterns' lookbehind/lookahead.

use std::sync::LazyLock;

use regex::{Regex, RegexBuilder};

/// Symbol-lookup queries: namespace-qualified, leading-underscore, or
/// containing uppercase/underscore. Plain lowercase words (e.g. `"session"`)
/// are NL, not symbols.
///
/// Equivalent to the upstream `SYMBOL_QUERY_RE`, with `\w`/`\d` written as
/// explicit ASCII classes and Unicode disabled so it matches the JavaScript
/// (ASCII `\w`) semantics exactly.
static SYMBOL_QUERY_RE: LazyLock<Regex> = LazyLock::new(|| {
    RegexBuilder::new(
        r"^(?:[A-Z_a-z][A-Za-z0-9_]*(?:(?:::|\\|->|\.)[A-Z_a-z][A-Za-z0-9_]*)+|_[A-Za-z0-9_]*|[A-Za-z][0-9a-z]*[A-Z_][A-Za-z0-9_]*|[A-Z][A-Za-z0-9]*)$",
    )
    .unicode(false)
    .build()
    .expect("SYMBOL_QUERY_RE is a valid regex")
});

/// Return true if the query looks like a bare symbol or namespace-qualified
/// identifier.
pub fn is_symbol_query(query: &str) -> bool {
    SYMBOL_QUERY_RE.is_match(query.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mirrors the isSymbolQuery suite in src/ranking/boosting.test.ts.

    #[test]
    fn pascal_case_is_symbol() {
        assert!(is_symbol_query("HandlerStack"));
        assert!(is_symbol_query("Client"));
    }

    #[test]
    fn namespace_qualified_is_symbol() {
        assert!(is_symbol_query("Sinatra::Base"));
        assert!(is_symbol_query("Phoenix.Router"));
        assert!(is_symbol_query("foo->bar"));
        assert!(is_symbol_query(r"A\B\C"));
    }

    #[test]
    fn leading_underscore_is_symbol() {
        assert!(is_symbol_query("_private"));
        assert!(is_symbol_query("_"));
    }

    #[test]
    fn snake_case_is_symbol() {
        assert!(is_symbol_query("my_func"));
    }

    #[test]
    fn plain_lowercase_words_are_nl() {
        assert!(!is_symbol_query("session"));
        assert!(!is_symbol_query("foo"));
    }

    #[test]
    fn nl_phrases_are_nl() {
        assert!(!is_symbol_query("how does this work"));
        assert!(!is_symbol_query("find the cache layer"));
    }

    #[test]
    fn trims_whitespace() {
        assert!(is_symbol_query("  HandlerStack  "));
    }
}
