import { useVirtualizer } from "@tanstack/react-virtual";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import type { Message, Space } from "../bridge-types";
import { loadOlder, markRead, ui } from "../actions";
import { useExit } from "../lib/motion";
import { dayLabel, parseTime, sameDay } from "../lib/time";
import { nameMap, spaceLabel, threadInfo, useStore } from "../store";
import { Icon } from "./icons";
import { MessageRow, type Reader } from "./MessageRow";
import { Welcome } from "./SpacePanels";

/** Where a message sits in its sender run: a block has one header (first or solo) and tight follow-ups (mid, last). */
export type Pos = "solo" | "first" | "mid" | "last";
type Row = { kind: "divider"; key: string; label: string } | { kind: "new"; key: string } | { kind: "msg"; key: string; m: Message; collapsed: boolean; pos: Pos };

const COLLAPSE_MS = 5 * 60_000;
const BOTTOM_PX = 48;

/** True when `m` continues `prev`'s run: same sender, within five minutes, same day, no attachment. */
export function continues(prev: Message | null, m: Message): boolean {
  if (!prev || !m.sender || prev.sender?.name !== m.sender.name || m.attachment?.length) return false;
  const a = parseTime(prev.createTime);
  const b = parseTime(m.createTime);
  return sameDay(a, b) && b.getTime() - a.getTime() < COLLAPSE_MS;
}

/** Block positions for a plain list (thread panel, feeds), with the same rule the main list uses. */
export function positions(items: Message[], breakBefore: (m: Message) => boolean = () => false): Pos[] {
  const cont = items.map((m, i) => !breakBefore(m) && continues(items[i - 1] ?? null, m));
  return items.map((_, i) => {
    const next = cont[i + 1] ?? false;
    return cont[i] ? (next ? "mid" : "last") : next ? "first" : "solo";
  });
}

/** `newFrom` is how many of the last top-level messages were unread when the space opened: the "New" line goes above them. */
export function buildRows(items: Message[], now = new Date(), newFrom = 0): Row[] {
  const rows: Row[] = [];
  let prevDate: Date | null = null;
  const top = items.filter((m) => !m.threadReply);
  const newAt = newFrom > 0 && newFrom <= top.length ? top[top.length - newFrom]!.name : null;
  const pos = positions(top, (m) => m.name === newAt);
  top.forEach((m, i) => {
    if (m.name === newAt) rows.push({ kind: "new", key: "new" });
    const d = parseTime(m.createTime);
    if (!prevDate || !sameDay(prevDate, d)) rows.push({ kind: "divider", key: `d:${d.toDateString()}`, label: dayLabel(d, now) });
    const p = pos[i]!;
    rows.push({ kind: "msg", key: m.name, m, collapsed: p === "mid" || p === "last", pos: p });
    prevDate = d;
  });
  return rows;
}

/** The day label that applies at row `index`: the nearest divider at or above it. */
export function dayAt(rows: Row[], index: number): string {
  for (let i = Math.min(index, rows.length - 1); i >= 0; i--) {
    const r = rows[i]!;
    if (r.kind === "divider") return r.label;
  }
  return "";
}

export function MessageList({ space }: { space: Space }) {
  const page = useStore((s) => s.messages[space.name]);
  const members = useStore((s) => s.members[space.name]);
  const me = useStore((s) => s.auth.user?.name);
  const receipts = useStore((s) => s.receipts[space.name]);
  const items = page?.items ?? EMPTY;
  // The unread count at open is frozen for this mount, so the "New" line stays put while the list is read.
  const [newFrom] = useState(() => useStore.getState().ui.unreadAtOpen);
  const rows = useMemo(() => buildRows(items, new Date(), newFrom), [items, newFrom]);
  const jump = useStore((s) => s.ui.jumpTo);
  const [flash, setFlash] = useState<string | null>(null);
  const threads = useMemo(() => threadInfo(items), [items]);
  const names = useMemo(() => nameMap(members), [members]);
  const parentRef = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);
  const prepend = useRef<{ scrollHeight: number; scrollTop: number } | null>(null);
  const rowCount = useRef(0);
  const [away, setAway] = useState(false);
  const [fresh, setFresh] = useState(0);
  const [date, setDate] = useState({ label: "", on: false });
  const dateTimer = useRef(0);
  const wheelAt = useRef(0);
  const jumpPill = useExit(away, 120);
  const firstKey = rows[0]?.key;
  const lastKey = rows[rows.length - 1]?.key;

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => parentRef.current,
    estimateSize: (i) => (rows[i]!.kind === "divider" ? 44 : rows[i]!.kind === "new" ? 28 : rows[i]!.kind === "msg" && rows[i]!.collapsed ? 26 : 66),
    overscan: 10,
    getItemKey: (i) => rows[i]!.key,
  });

  // Who has read up to where: each other reader's receipt sits under the message it names (relay receipts from
  // other Confab clients; Google does not expose other people's read state to user auth).
  const readers = useMemo(() => {
    const out: Record<string, Reader[]> = {};
    if (!receipts) return out;
    const known = new Set(items.map((m) => m.name));
    for (const [user, r] of Object.entries(receipts)) {
      if (user === me || !known.has(r.message)) continue;
      (out[r.message] ??= []).push({ user: names[user] ?? { name: user }, at: r.at });
    }
    return out;
  }, [receipts, items, names, me]);

  const toBottom = useCallback((smooth = false) => {
    if (!rows.length) return;
    atBottom.current = true;
    setAway(false);
    setFresh(0);
    virtualizer.scrollToIndex(rows.length - 1, { align: "end", behavior: smooth ? "smooth" : "auto" });
    if (smooth) return;
    // Measured heights replace estimates a frame later, so pin the bottom now and again after that frame
    // (a hidden document gets no frames at all, which is why the synchronous pin matters).
    const el = parentRef.current;
    if (el) el.scrollTop = el.scrollHeight;
    requestAnimationFrame(() => {
      const e = parentRef.current;
      if (e && atBottom.current) e.scrollTop = e.scrollHeight;
    });
  }, [rows.length, virtualizer]);

  // Initial + stick-to-bottom. A message of mine pulls the view down even if I had scrolled up.
  useLayoutEffect(() => {
    const el = parentRef.current;
    if (!el) return;
    if (prepend.current) {
      el.scrollTop = el.scrollHeight - prepend.current.scrollHeight + prepend.current.scrollTop;
      prepend.current = null;
      rowCount.current = rows.length;
      return;
    }
    const last = rows[rows.length - 1];
    const ownNew = last?.kind === "msg" && last.m.sender?.name === me && !!last.m._pending;
    if (!atBottom.current && rows.length > rowCount.current && rowCount.current > 0 && !ownNew) setFresh((f) => f + (rows.length - rowCount.current));
    rowCount.current = rows.length;
    if ((atBottom.current || ownNew) && rows.length) toBottom();
  }, [rows, lastKey, firstKey, space.name, me, toBottom]);
  useEffect(() => {
    atBottom.current = true;
    rowCount.current = 0;
    setAway(false);
    setFresh(0);
  }, [space.name]);

  const maybeMarkRead = useCallback(() => {
    if (!atBottom.current || !document.hasFocus()) return;
    const last = [...items].reverse().find((m) => !m.threadReply && !m._pending);
    if (last) markRead(space.name, last.name);
  }, [items, space.name]);
  useEffect(() => {
    maybeMarkRead();
    window.addEventListener("focus", maybeMarkRead);
    return () => window.removeEventListener("focus", maybeMarkRead);
  }, [maybeMarkRead]);

  const onScroll = () => {
    const el = parentRef.current;
    if (!el) return;
    const bottom = el.scrollHeight - el.scrollTop - el.clientHeight < BOTTOM_PX;
    atBottom.current = bottom;
    setAway(!bottom);
    if (bottom) { setFresh(0); maybeMarkRead(); }
    // Floating day chip: the day of the topmost visible row, shown while the user scrolls (not on programmatic jumps).
    const first = virtualizer.getVirtualItems().find((v) => v.end > el.scrollTop + 6);
    if (first && el.scrollTop > 40 && Date.now() - wheelAt.current < 400) {
      setDate({ label: dayAt(rows, first.index), on: true });
      window.clearTimeout(dateTimer.current);
      dateTimer.current = window.setTimeout(() => setDate((d) => ({ ...d, on: false })), 900);
    } else setDate((d) => (d.on ? { ...d, on: false } : d));
    if (el.scrollTop < 240 && page?.nextPageToken && !page.loading && !prepend.current) {
      prepend.current = { scrollHeight: el.scrollHeight, scrollTop: el.scrollTop };
      void loadOlder(space.name).then((ok) => {
        if (!ok) prepend.current = null;
      });
    }
  };
  useEffect(() => () => window.clearTimeout(dateTimer.current), []);
  // A panel opening or the window resizing reflows the list; a view that was at the bottom stays there.
  useEffect(() => {
    const el = parentRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => { if (atBottom.current) el.scrollTop = el.scrollHeight; });
    ro.observe(el);
    return () => ro.disconnect();
  }, [page?.loaded]);

  // A jump (saved item, search hit, unreads) scrolls the row into the middle and flashes it; older pages load until it is found.
  useEffect(() => {
    if (!jump || !page?.loaded) return;
    const i = rows.findIndex((r) => r.kind === "msg" && r.m.name === jump.message);
    if (i >= 0) {
      ui({ jumpTo: undefined });
      atBottom.current = false;
      virtualizer.scrollToIndex(i, { align: "center" });
      requestAnimationFrame(() => virtualizer.scrollToIndex(i, { align: "center" }));
      setFlash(jump.message);
      window.setTimeout(() => setFlash(null), 1600);
      return;
    }
    const el = parentRef.current;
    if (page.nextPageToken && !page.loading && Date.now() - jump.at < 8000 && el) {
      atBottom.current = false;
      prepend.current = { scrollHeight: el.scrollHeight, scrollTop: el.scrollTop };
      void loadOlder(space.name).then((ok) => { if (!ok) prepend.current = null; });
    } else if (!page.nextPageToken || Date.now() - jump.at >= 8000) ui({ jumpTo: undefined });
  }, [jump, rows, page?.loaded, page?.nextPageToken, page?.loading, space.name, virtualizer]);

  const label = spaceLabel(space, members, me);
  if (!page?.loaded) {
    return (
      <div className="listwrap">
        <div className="list" role="list" aria-label="Messages" aria-busy="true">
          <Skeleton />
        </div>
      </div>
    );
  }
  if (rows.length === 0) return <div className="listwrap"><Welcome space={space} /></div>;
  return (
    <div className="listwrap">
      <div ref={parentRef} className="list" role="list" aria-label="Messages" onScroll={onScroll} onWheel={() => { wheelAt.current = Date.now(); }}>
        <div className="list__inner" style={{ height: virtualizer.getTotalSize() }}>
          {virtualizer.getVirtualItems().map((v) => {
            const row = rows[v.index]!;
            return (
              <div key={row.key} className="list__row" data-index={v.index} ref={virtualizer.measureElement} style={{ transform: `translateY(${v.start}px)` }}>
                {v.index === 0 && (
                  <div className="list__top">
                    {page?.loading ? "Loading older messages…" : page?.nextPageToken ? "Scroll up for more" : space.spaceType === "SPACE" ? `This is the start of #${label}.` : `This is the start of your conversation with ${label}.`}
                  </div>
                )}
                {row.kind === "divider" ? (
                  <div className="divider" role="separator" aria-label={row.label}><span>{row.label}</span></div>
                ) : row.kind === "new" ? (
                  <div className="divider divider--new" role="separator" aria-label="New messages"><span>New</span></div>
                ) : (
                  <MessageRow m={row.m} collapsed={row.collapsed} pos={row.pos} me={me} members={names} thread={row.m.thread?.name ? threads[row.m.thread.name] : undefined} readers={readers[row.m.name]} isDm={space.spaceType === "DIRECT_MESSAGE"} flash={flash === row.m.name} />
                )}
              </div>
            );
          })}
        </div>
      </div>
      <div className={`list__date${date.on ? " list__date--on" : ""}`} aria-hidden="true">{date.label}</div>
      {jumpPill.mounted && (
        <button type="button" className={`list__jump${jumpPill.exiting ? " list__jump--out" : ""}`} onClick={() => toBottom(true)}>
          <Icon n="arrowdown" />
          {fresh > 0 ? `${fresh} new ${fresh === 1 ? "message" : "messages"}` : "Jump to latest"}
        </button>
      )}
    </div>
  );
}
const EMPTY: Message[] = [];

const WIDTHS = [[42, 70], [28], [55, 80, 33], [36], [48, 62], [25], [60, 44]];
/** Placeholder rows while the first page loads: shimmering bars where names and lines will be. */
function Skeleton() {
  return (
    <div aria-hidden="true">
      {WIDTHS.map((w, i) => (
        <div key={i} className="skel" style={{ animationDelay: `${i * 40}ms` }}>
          <div className="skel__av" />
          <div className="skel__body">
            <div className="skel__line" style={{ width: "22%" }} />
            {w.map((p, j) => <div key={j} className="skel__line" style={{ width: `${p}%` }} />)}
          </div>
        </div>
      ))}
    </div>
  );
}
