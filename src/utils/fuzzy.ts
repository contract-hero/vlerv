// Subsequence fuzzy scorer for the quick-open palette. No dependency; ~40
// lines. Higher score = better. Returns null when the query is not a
// subsequence of the candidate.
export function fuzzyScore(query: string, candidate: string): number | null {
  if (!query) return 0;
  const q = query.toLowerCase();
  const c = candidate.toLowerCase();

  let qi = 0;
  let score = 0;
  let lastMatch = -1;

  for (let ci = 0; ci < c.length && qi < q.length; ci++) {
    if (c[ci] !== q[qi]) continue;

    // Base point per matched char.
    let charScore = 1;
    // Bonus: match at a path-segment or word boundary.
    const prev = ci > 0 ? c[ci - 1] : "";
    if (ci === 0 || prev === "/" || prev === "-" || prev === "_" || prev === ".") {
      charScore += 8;
    }
    // Bonus: consecutive matches.
    if (lastMatch === ci - 1) {
      charScore += 4;
    }
    // Penalty: gap since the previous match.
    if (lastMatch >= 0) {
      charScore -= Math.min(3, (ci - lastMatch - 1) * 0.2);
    }
    score += charScore;
    lastMatch = ci;
    qi++;
  }

  if (qi < q.length) return null;

  // Prefer matches concentrated in the basename and shorter paths overall.
  const basenameStart = c.lastIndexOf("/") + 1;
  if (lastMatch >= basenameStart) score += 6;
  score -= c.length * 0.01;
  return score;
}

export interface FuzzyResult {
  value: string;
  score: number;
}

export function fuzzyFilter(query: string, candidates: readonly string[], limit: number): FuzzyResult[] {
  const results: FuzzyResult[] = [];
  for (const value of candidates) {
    const score = fuzzyScore(query, value);
    if (score !== null) results.push({ value, score });
  }
  results.sort((a, b) => b.score - a.score);
  return results.slice(0, limit);
}
