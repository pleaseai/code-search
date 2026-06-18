// Port of src/semble/tokens.py

const TOKEN_RE = /[a-z_]\w*/gi

// Split on camelCase/PascalCase boundaries:
//   "HandlerStack" -> ["Handler", "Stack"]
//   "getHTTPResponse" -> ["get", "HTTP", "Response"]
//   "XMLParser" -> ["XML", "Parser"]
const CAMEL_RE = /[A-Z]+(?=[A-Z][a-z])|[A-Z]?[a-z]+|[A-Z]+|\d+/g

/**
 * Split a single identifier into sub-tokens via camelCase/snake_case.
 *
 * Returns the original token (lowered) plus any sub-tokens.
 * E.g. "HandlerStack" -> ["handlerstack", "handler", "stack"]
 *      "my_func" -> ["my_func", "my", "func"]
 *      "simple" -> ["simple"]
 */
export function splitIdentifier(token: string): string[] {
  const lower = token.toLowerCase()

  // Fast-path: a token made up solely of lowercase ASCII letters cannot split
  // further, since `CAMEL_RE` would match it as a single run. This guard is
  // intentionally narrow — `splitIdentifier` is also called on raw path stems
  // (e.g. "user-service", "foo.bar"), and `CAMEL_RE` treats `-`/`.` as
  // separators, so those must fall through to the splitting logic below.
  if (/^[a-z]+$/.test(token)) {
    return [lower]
  }

  let parts: string[]

  if (token.includes('_')) {
    // snake_case splitting
    parts = lower.split('_').filter(p => p.length > 0)
  }
  else {
    // camelCase / PascalCase splitting
    parts = Array.from(token.matchAll(CAMEL_RE), ([m]) => m.toLowerCase())
  }

  if (parts.length >= 2) {
    return [lower, ...parts]
  }
  return [lower]
}

/**
 * Split text into lowercase identifier-like tokens for BM25 indexing.
 *
 * Compound identifiers (camelCase, PascalCase, snake_case) are expanded
 * into sub-tokens so that partial matches work. The original compound
 * token is preserved for exact-match boosting.
 */
export function tokenize(text: string): string[] {
  const result: string[] = []
  for (const [match] of text.matchAll(TOKEN_RE)) {
    result.push(...splitIdentifier(match))
  }
  return result
}
