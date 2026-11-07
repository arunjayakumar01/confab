import { useEffect, useRef, useState } from "react";
import { useExit } from "../lib/motion";
import { typingText } from "../lib/typing";
import { nameMap, useStore } from "../store";

/** "Priya is typing…" under a list. For the main list only space-level typing; for a thread only that thread. */
export function TypingStrip({ space, thread }: { space: string; thread?: string }) {
  const typing = useStore((s) => s.typing[space]);
  const members = useStore((s) => s.members[space]);
  const [, tick] = useState(0);
  useEffect(() => {
    const t = window.setInterval(() => tick((n) => n + 1), 1000);
    return () => window.clearInterval(t);
  }, []);
  const now = Date.now();
  const names = nameMap(members);
  const who = Object.entries(typing ?? {})
    .filter(([, e]) => e.until > now && (thread ? e.thread === thread : !e.thread))
    .map(([u]) => names[u]?.displayName ?? "Someone");
  const text = typingText(who);
  const { mounted, exiting } = useExit(!!text, 120);
  const last = useRef(text);
  if (text) last.current = text;
  return (
    <div className="typing" aria-live="polite">
      {mounted && (
        <span className={`typing__in${exiting ? " typing__in--out" : ""}`}>
          <span className="typing__dots" aria-hidden="true"><i /><i /><i /></span>
          {last.current}
        </span>
      )}
    </div>
  );
}
