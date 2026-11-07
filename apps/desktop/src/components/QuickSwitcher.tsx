import { useEffect, useMemo, useState } from "react";
import type { Space, User } from "../bridge-types";
import { openDm, openSpace, ui } from "../actions";
import { fuzzyScore } from "../lib/fuzzy";
import { dmPeer, spaceLabel, useStore } from "../store";
import { Avatar, Presence } from "./Avatar";
import { Icon } from "./icons";
import { Modal } from "./Modal";
import { usePeopleSearch, userLabel } from "./PeoplePicker";

interface Item { key: string; label: string; kind: string; space?: Space; user?: User }

export function QuickSwitcher({ exiting }: { exiting?: boolean }) {
  const spaces = useStore((s) => s.spaces);
  const members = useStore((s) => s.members);
  const presence = useStore((s) => s.presence);
  const me = useStore((s) => s.auth.user?.name);
  const [q, setQ] = useState("");
  const [idx, setIdx] = useState(0);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const { hits: directory, busy: searching } = usePeopleSearch(q);
  const close = () => ui({ switcher: false });

  const all = useMemo(() => {
    const out: Item[] = [];
    const dmOf = new Map<string, Space>();
    for (const s of spaces) {
      const peer = dmPeer(s, members[s.name], me);
      if (peer) dmOf.set(peer.name, s);
      out.push({ key: s.name, label: spaceLabel(s, members[s.name], me), kind: s.spaceType === "SPACE" ? "Channel" : s.spaceType === "GROUP_CHAT" ? "Group" : "Direct message", space: s, user: peer });
    }
    const seen = new Set<string>();
    for (const list of Object.values(members)) for (const m of list) {
      const u = m.member;
      if (!u || u.name === me || seen.has(u.name) || dmOf.has(u.name)) continue;
      seen.add(u.name);
      out.push({ key: u.name, label: u.displayName ?? u.name, kind: "No DM yet", user: u });
    }
    return out;
  }, [spaces, members, me]);

  const list = useMemo(() => {
    const local = !q.trim()
      ? all.slice(0, 12)
      : all.map((it) => ({ it, s: fuzzyScore(q, it.label) })).filter((x) => x.s >= 0).sort((a, b) => b.s - a.s).slice(0, 12).map((x) => x.it);
    const known = new Set(all.flatMap((it) => (it.user ? [it.user.name] : [])));
    const remote = directory.filter((u) => u.name !== me && !known.has(u.name)).map((u): Item => ({ key: `dir:${u.name}`, label: userLabel(u), kind: "Directory", user: u }));
    return [...local, ...remote];
  }, [all, q, directory, me]);
  useEffect(() => setIdx(0), [q]);

  const pick = (it: Item | undefined) => {
    if (!it || busy) return;
    if (it.space) { openSpace(it.space.name); close(); return; }
    if (!it.user) return;
    setBusy(true);
    setErr("");
    openDm(it.user).then(close).catch((e: unknown) => { setErr((e as Error)?.message ?? String(e)); setBusy(false); });
  };
  return (
    <Modal onClose={close} title="Jump to" bare exiting={exiting}>
      <input
        className="dialog__input"
        autoFocus
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder="Jump to a channel or person"
        aria-label="Jump to a channel or person"
        role="combobox"
        aria-expanded="true"
        aria-controls="switcher-list"
        aria-busy={busy || searching}
        onKeyDown={(e) => {
          if (e.key === "ArrowDown") { e.preventDefault(); setIdx((i) => Math.min(i + 1, list.length - 1)); }
          else if (e.key === "ArrowUp") { e.preventDefault(); setIdx((i) => Math.max(i - 1, 0)); }
          else if (e.key === "Enter") { e.preventDefault(); pick(list[idx]); }
        }}
      />
      <div className="dialog__list" id="switcher-list" role="listbox">
        {list.map((it, i) => (
          <button key={it.key} type="button" role="option" aria-selected={i === idx} className={`dialog__item${i === idx ? " dialog__item--on" : ""}`} onMouseEnter={() => setIdx(i)} onClick={() => pick(it)} disabled={busy}>
            {it.user ? <Avatar user={it.user} size={22} /> : <span className="row__icon">{it.space?.spaceType === "GROUP_CHAT" ? <Icon n="users" /> : "#"}</span>}
            {it.user && <Presence state={presence[it.user.name]?.state ?? "offline"} />}
            <span>{it.label}</span>
            {!it.space && it.user?.email && <span className="dialog__sub">{it.user.email}</span>}
            <span className="dialog__kind">{it.kind}</span>
          </button>
        ))}
        {list.length === 0 && <div className="dialog__empty">{searching ? "Searching the directory…" : `Nothing matches "${q}"`}</div>}
      </div>
      {(busy || err) && <div className={`dialog__note${err ? " dialog__note--err" : ""}`} role="status">{err || "Starting conversation…"}</div>}
      <div className="dialog__foot">
        <span><span className="kbd">↑↓</span> move</span>
        <span><span className="kbd">↵</span> open</span>
        <span><span className="kbd">esc</span> close</span>
      </div>
    </Modal>
  );
}
