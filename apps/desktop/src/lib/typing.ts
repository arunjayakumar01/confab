/** Typing strip text. Names are display names of users currently typing. */
export function typingText(names: string[]): string | null {
  const n = names.filter(Boolean);
  if (n.length === 0) return null;
  if (n.length === 1) return `${n[0]} is typing…`;
  if (n.length === 2) return `${n[0]} and ${n[1]} are typing…`;
  return "Several people are typing…";
}
