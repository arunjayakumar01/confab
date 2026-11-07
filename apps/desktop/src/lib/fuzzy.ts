/** Subsequence match; higher is better, -1 means no match. Prefers prefixes and word starts. */
export function fuzzyScore(query: string, target: string): number {
  const q = query.toLowerCase().trim();
  const t = target.toLowerCase();
  if (!q) return 0;
  if (t.startsWith(q)) return 100 + q.length * 2;
  const at = t.indexOf(q);
  if (at >= 0) return 60 + (t[at - 1] === " " ? 20 : 0) - at * 0.1;
  let qi = 0;
  let score = 0;
  let last = -2;
  for (let i = 0; i < t.length && qi < q.length; i++) {
    if (t[i] === q[qi]) {
      score += i === last + 1 ? 4 : i === 0 || t[i - 1] === " " ? 3 : 1;
      last = i;
      qi++;
    }
  }
  return qi === q.length ? score : -1;
}
