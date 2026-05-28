// Port of src/semble/ranking/weighting.py

import { isSymbolQuery } from './boosting.ts'

export const ALPHA_SYMBOL = 0.3 // lean BM25 for exact keyword matching
export const ALPHA_NL = 0.5 // balanced semantic + BM25

/** Return the blending weight for semantic scores, auto-detecting from query type. */
export function resolveAlpha(query: string, alpha: number | null | undefined): number {
  if (alpha !== null && alpha !== undefined) {
    return alpha
  }
  return isSymbolQuery(query) ? ALPHA_SYMBOL : ALPHA_NL
}
