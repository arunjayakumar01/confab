import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { EMOJI, searchEmoji, type EmojiEntry } from "../lib/emoji";
import { useStore } from "../store";

export interface Anchor { x: number; y: number }

const W = 300;
const H = 290;

/**
 * Fixed-position picker anchored at a point, portalled to body: message rows live inside transformed,
 * overflow-clipped virtual rows, where `position: fixed` would be clipped and offset. Closes on outside click or Esc;
 * the host keeps it mounted through `useExit` and passes `exiting` for the close animation.
 */
export function EmojiPicker({ at, onPick, onClose, exiting }: { at: Anchor; onPick: (u: string) => void; onClose: () => void; exiting?: boolean }) {
  const [q, setQ] = useState("");
  const ref = useRef<HTMLDivElement>(null);
  const recentU = useStore((s) => s.recentEmoji);
  const searching = !!q.trim();
  const recent: EmojiEntry[] = searching ? [] : recentU.map((u) => EMOJI.find((e) => e.u === u) ?? { u, n: u });
  const list = searching ? searchEmoji(q) : EMOJI.filter((e) => !recentU.includes(e.u));
  const first = recent[0] ?? list[0];
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [onClose]);
  const left = Math.max(8, Math.min(at.x - W / 2, window.innerWidth - W - 8));
  const up = at.y + H > window.innerHeight - 8;
  const top = up ? Math.max(8, at.y - H - 8) : at.y + 8;
  const grid = (items: EmojiEntry[]) => (
    <div className="emoji__grid">
      {items.map((e) => (
        <button key={e.u} type="button" title={e.n} aria-label={e.n} onClick={() => onPick(e.u)}>
          {e.u}
        </button>
      ))}
    </div>
  );
  return createPortal(
    <div
      ref={ref}
      className={`emoji ${up ? "emoji--up" : "emoji--down"}${exiting ? " emoji--out" : ""}`}
      style={{ left, top }}
      role="dialog"
      aria-label="Pick an emoji"
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          e.stopPropagation();
          onClose();
        }
        if (e.key === "Enter" && first) {
          e.preventDefault();
          onPick(first.u);
        }
      }}
    >
      <input autoFocus value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search emoji" aria-label="Search emoji" />
      <div className="emoji__scroll">
        {recent.length > 0 && (
          <>
            <div className="emoji__label">Frequently used</div>
            {grid(recent)}
            <div className="emoji__label">All</div>
          </>
        )}
        {grid(list)}
        {list.length === 0 && <div className="emoji__none">Nothing matches "{q}"</div>}
      </div>
    </div>,
    document.body,
  );
}
