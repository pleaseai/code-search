//! Semantic/BM25 blending weight. Port of `src/ranking/weighting.ts`
//! (← semble `ranking/weighting.py`).

use super::boosting::is_symbol_query;

/// Lean BM25 for exact keyword matching.
pub const ALPHA_SYMBOL: f64 = 0.3;
/// Balanced semantic + BM25.
pub const ALPHA_NL: f64 = 0.5;

/// Return the blending weight for semantic scores, auto-detecting from query
/// type when `alpha` is `None`. An explicit `Some(0.0)` is honored (not treated
/// as missing), matching the TypeScript `null`/`undefined` distinction.
pub fn resolve_alpha(query: &str, alpha: Option<f64>) -> f64 {
    match alpha {
        Some(value) => value,
        None => {
            if is_symbol_query(query) {
                ALPHA_SYMBOL
            } else {
                ALPHA_NL
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Mirrors src/ranking/weighting.test.ts.

    #[test]
    fn returns_nl_for_plain_lowercase_queries() {
        assert_eq!(resolve_alpha("session", None), 0.5);
        assert_eq!(resolve_alpha("session", None), ALPHA_NL);
    }

    #[test]
    fn returns_symbol_for_pascal_case_queries() {
        assert_eq!(resolve_alpha("HandlerStack", None), 0.3);
        assert_eq!(resolve_alpha("HandlerStack", None), ALPHA_SYMBOL);
    }

    #[test]
    fn returns_provided_alpha_when_set() {
        assert_eq!(resolve_alpha("foo", Some(0.7)), 0.7);
        assert_eq!(resolve_alpha("HandlerStack", Some(0.9)), 0.9);
    }

    #[test]
    fn alpha_zero_is_honored() {
        assert_eq!(resolve_alpha("HandlerStack", Some(0.0)), 0.0);
    }
}
