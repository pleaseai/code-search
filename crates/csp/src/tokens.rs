//! Identifier-aware tokenization. Port of `src/tokens.ts` (← semble `tokens.py`).
//!
//! Behavioral equivalence with the TypeScript implementation is verified against
//! the same test vectors (see the test module). The upstream `CAMEL_RE` uses a
//! regex lookahead (`(?=[A-Z][a-z])`), which the Rust `regex` crate does not
//! support; the camelCase splitter is reimplemented here as a state machine that
//! reproduces the regex's match sequence exactly (and runs faster on the hot
//! indexing path).

/// Split a single identifier into sub-tokens via camelCase/snake_case.
///
/// Returns the original token (lowered) plus any sub-tokens. E.g.
/// `"HandlerStack"` → `["handlerstack", "handler", "stack"]`,
/// `"my_func"` → `["my_func", "my", "func"]`, `"simple"` → `["simple"]`.
pub fn split_identifier(token: &str) -> Vec<String> {
    let lower = token.to_ascii_lowercase();

    // Fast-path: a pure-lowercase token with no underscores or digits cannot
    // split further. Token chars are always ASCII `[A-Za-z0-9_]` (see TOKEN_RE
    // in `tokenize`), so the absence of `_`, uppercase, and digits means the
    // token is already a single sub-token.
    let has_underscore = token.contains('_');
    let has_upper_or_digit = token
        .bytes()
        .any(|b| b.is_ascii_uppercase() || b.is_ascii_digit());
    if !has_underscore && !has_upper_or_digit {
        return vec![lower];
    }

    let parts: Vec<String> = if has_underscore {
        // snake_case: split the *lowered* string on `_`, dropping empties
        // (mirrors Python `split('_')` + filter for consecutive underscores).
        lower
            .split('_')
            .filter(|p| !p.is_empty())
            .map(str::to_string)
            .collect()
    } else {
        // camelCase / PascalCase splitting over the *original* token.
        camel_split(token)
            .into_iter()
            .map(str::to_ascii_lowercase)
            .collect()
    };

    if parts.len() >= 2 {
        let mut out = Vec::with_capacity(parts.len() + 1);
        out.push(lower);
        out.extend(parts);
        out
    } else {
        vec![lower]
    }
}

/// Reproduce `matchAll(/[A-Z]+(?=[A-Z][a-z])|[A-Z]?[a-z]+|[A-Z]+|\d+/g)` over an
/// ASCII identifier (no underscores — those take the snake_case path).
fn camel_split(token: &str) -> Vec<&str> {
    let b = token.as_bytes();
    let n = b.len();
    let mut out = Vec::new();
    let mut p = 0;
    while p < n {
        let c = b[p];
        if c.is_ascii_uppercase() {
            // Maximal run of uppercase starting at p.
            let mut q = p;
            while q < n && b[q].is_ascii_uppercase() {
                q += 1;
            }
            let run = q - p;
            let next_is_lower = q < n && b[q].is_ascii_lowercase();
            if run >= 2 && next_is_lower {
                // alt 1: `[A-Z]+(?=[A-Z][a-z])` — greedy capitals leave the last
                // one to start the following lowercase word.
                out.push(&token[p..q - 1]);
                p = q - 1;
            } else if run == 1 && next_is_lower {
                // alt 2: `[A-Z]?[a-z]+` — one capital + its lowercase run.
                let mut r = q;
                while r < n && b[r].is_ascii_lowercase() {
                    r += 1;
                }
                out.push(&token[p..r]);
                p = r;
            } else {
                // alt 3: `[A-Z]+` — capital run not followed by a lowercase
                // (end of token, or a digit run).
                out.push(&token[p..q]);
                p = q;
            }
        } else if c.is_ascii_lowercase() {
            // alt 2 with no leading capital: a bare lowercase run.
            let mut r = p;
            while r < n && b[r].is_ascii_lowercase() {
                r += 1;
            }
            out.push(&token[p..r]);
            p = r;
        } else if c.is_ascii_digit() {
            // alt 4: `\d+`.
            let mut r = p;
            while r < n && b[r].is_ascii_digit() {
                r += 1;
            }
            out.push(&token[p..r]);
            p = r;
        } else {
            // Unreachable for camel tokens (all chars are ASCII alphanumeric),
            // but advance defensively rather than loop forever.
            p += 1;
        }
    }
    out
}

/// Split text into lowercase identifier-like tokens for BM25 indexing.
///
/// Compound identifiers (camelCase, PascalCase, snake_case) are expanded into
/// sub-tokens so partial matches work; the original compound token is preserved
/// for exact-match boosting.
pub fn tokenize(text: &str) -> Vec<String> {
    let mut result = Vec::new();
    for token in token_matches(text) {
        result.extend(split_identifier(token));
    }
    result
}

/// Reproduce `matchAll(/[a-z_]\w*/gi)`: maximal runs that start with an ASCII
/// letter or `_` and continue with ASCII letters, digits, or `_`. A run cannot
/// start with a digit, so bare numbers (e.g. `"123"`) are not matched.
fn token_matches(text: &str) -> Vec<&str> {
    let b = text.as_bytes();
    let n = b.len();
    let mut out = Vec::new();
    let mut p = 0;
    while p < n {
        if b[p].is_ascii_alphabetic() || b[p] == b'_' {
            let mut q = p + 1;
            while q < n && (b[q].is_ascii_alphanumeric() || b[q] == b'_') {
                q += 1;
            }
            out.push(&text[p..q]);
            p = q;
        } else {
            p += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mirrors src/tokens.test.ts (golden fixtures from the TypeScript suite).

    #[test]
    fn splits_pascal_case() {
        assert_eq!(
            split_identifier("HandlerStack"),
            ["handlerstack", "handler", "stack"]
        );
    }

    #[test]
    fn preserves_runs_of_capitals_as_a_single_sub_token() {
        assert_eq!(
            split_identifier("getHTTPResponse"),
            ["gethttpresponse", "get", "http", "response"]
        );
    }

    #[test]
    fn handles_leading_run_of_capitals() {
        assert_eq!(
            split_identifier("XMLParser"),
            ["xmlparser", "xml", "parser"]
        );
    }

    #[test]
    fn splits_snake_case() {
        assert_eq!(split_identifier("my_func"), ["my_func", "my", "func"]);
    }

    #[test]
    fn returns_only_lowered_token_when_no_boundary() {
        assert_eq!(split_identifier("simple"), ["simple"]);
    }

    #[test]
    fn lowercases_an_already_lowercase_token() {
        assert_eq!(split_identifier("Already"), ["already"]);
    }

    #[test]
    fn keeps_consecutive_underscores_from_collapsing() {
        assert_eq!(split_identifier("foo__bar"), ["foo__bar", "foo", "bar"]);
    }

    #[test]
    fn treats_leading_underscore_as_one_effective_part() {
        assert_eq!(split_identifier("_foo"), ["_foo"]);
    }

    #[test]
    fn splits_digit_runs_as_their_own_camel_sub_token() {
        assert_eq!(
            split_identifier("abc123Def"),
            ["abc123def", "abc", "123", "def"]
        );
    }

    #[test]
    fn tokenize_splits_plain_space_separated_words() {
        assert_eq!(tokenize("foo bar baz"), ["foo", "bar", "baz"]);
    }

    #[test]
    fn tokenize_expands_compounds_and_drops_non_identifier_digits() {
        assert_eq!(
            tokenize("camelCase_snake_case 123"),
            ["camelcase_snake_case", "camelcase", "snake", "case"]
        );
    }

    #[test]
    fn tokenize_returns_empty_for_no_identifiers() {
        assert_eq!(tokenize("   !!! 123 ???"), Vec::<String>::new());
    }

    #[test]
    fn tokenize_preserves_multiple_identifiers_and_expands_each() {
        assert_eq!(
            tokenize("HandlerStack my_func"),
            ["handlerstack", "handler", "stack", "my_func", "my", "func"]
        );
    }
}
