const normalize = (text: string) => text.normalize("NFD").replace(/\p{M}/gu, "").toLowerCase();

const WORD_START = /[\s›/\\_.:-]/;

/**
 * Subsequence fuzzy match, case- and accent-insensitive; spaces in the query are ignored.
 * Higher is better; null when the text lacks a query character. Bonuses: consecutive chars, word starts, substring.
 */
export function fuzzyScore(query: string, text: string): number | null {
  const q = normalize(query).replace(/\s+/g, "");
  if (!q) return 0;
  const t = normalize(text);
  let score = 0;
  let from = 0;
  let prev = -2;
  for (const ch of q) {
    const at = t.indexOf(ch, from);
    if (at === -1) return null;
    score += 1;
    if (at === prev + 1) score += 3;
    if (at === 0 || WORD_START.test(t[at - 1])) score += 2;
    prev = at;
    from = at + 1;
  }
  if (t.includes(q)) score += q.length * 2;
  return score - t.length * 0.01; // shorter text wins ties
}
