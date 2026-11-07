import { useEffect, useMemo, useState } from "react";
import type { User } from "../bridge-types";
import { searchPeople } from "../actions";
import { fuzzyScore } from "../lib/fuzzy";
import { useStore } from "../store";
import { Avatar } from "./Avatar";
import { Icon } from "./icons";

export const userLabel = (u: User): string => u.displayName ?? u.email ?? u.name;

const DIRECTORY_MS = 8000;

/**
 * People from the spaces already loaded answer at once; the Workspace directory follows, debounced 250 ms from the
 * second character. A directory that fails or does not answer in time marks `failed` so the box can say so instead of
 * spinning (the Workspace admin "Directory sharing" setting decides whether it answers at all).
 */
export function usePeopleSearch(q: string): { hits: User[]; busy: boolean; failed: boolean } {
  const members = useStore((s) => s.members);
  const me = useStore((s) => s.auth.user?.name);
  const [remote, setRemote] = useState<User[]>([]);
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const local = useMemo(() => {
    const query = q.trim();
    if (query.length < 2) return [];
    const seen = new Set<string>();
    const out: { u: User; s: number }[] = [];
    for (const list of Object.values(members)) for (const m of list) {
      const u = m.member;
      if (!u || u.name === me || u.type === "BOT" || seen.has(u.name)) continue;
      seen.add(u.name);
      const s = Math.max(fuzzyScore(query, u.displayName ?? ""), fuzzyScore(query, u.email ?? ""));
      if (s >= 0) out.push({ u, s });
    }
    return out.sort((a, b) => b.s - a.s).slice(0, 8).map((x) => x.u);
  }, [q, members, me]);
  useEffect(() => {
    const query = q.trim();
    if (query.length < 2) { setRemote([]); setBusy(false); setFailed(false); return; }
    let live = true;
    setBusy(true);
    setFailed(false);
    const t = window.setTimeout(() => {
      const timeout = new Promise<never>((_, rej) => window.setTimeout(() => rej(new Error("directory timeout")), DIRECTORY_MS));
      Promise.race([searchPeople(query), timeout])
        .then((r) => { if (live) { setRemote(r); setBusy(false); } })
        .catch((e: unknown) => { console.warn("searchPeople", e); if (live) { setRemote([]); setFailed(true); setBusy(false); } });
    }, 250);
    return () => { live = false; window.clearTimeout(t); };
  }, [q]);
  const hits = useMemo(() => {
    const names = new Set(local.map((u) => u.name));
    return [...local, ...remote.filter((u) => !names.has(u.name))];
  }, [local, remote]);
  return { hits, busy, failed };
}

/** Chips of chosen people plus a directory search box. Never offers me or someone already chosen. */
export function PeoplePicker({ value, onChange, autoFocus }: { value: User[]; onChange: (v: User[]) => void; autoFocus?: boolean }) {
  const me = useStore((s) => s.auth.user?.name);
  const [q, setQ] = useState("");
  const { hits, busy, failed } = usePeopleSearch(q);
  const open = hits.filter((u) => u.name !== me && !value.some((v) => v.name === u.name));
  const asked = q.trim().length >= 2;
  const add = (u: User) => { onChange([...value, u]); setQ(""); };
  return (
    <div className="picker">
      <div className="chips">
        {value.map((u) => (
          <span key={u.name} className="chip">
            <Avatar user={u} size={18} />
            {userLabel(u)}
            <button type="button" aria-label={`Remove ${userLabel(u)}`} onClick={() => onChange(value.filter((v) => v.name !== u.name))}><Icon n="x" size={12} /></button>
          </span>
        ))}
        <input
          value={q}
          autoFocus={autoFocus}
          onChange={(e) => setQ(e.target.value)}
          placeholder={value.length ? "Add more people" : "Search people by name or email"}
          aria-label="Search people"
          onKeyDown={(e) => {
            if (e.key === "Enter") { e.preventDefault(); if (open[0]) add(open[0]); }
            else if (e.key === "Backspace" && !q && value.length) onChange(value.slice(0, -1));
          }}
        />
      </div>
      {asked && (
        <div className="picker__hits" role="listbox" aria-label="People" aria-busy={busy}>
          {open.map((u) => (
            <button key={u.name} type="button" role="option" aria-selected={false} className="dialog__item" onClick={() => add(u)}>
              <Avatar user={u} size={22} />
              <span>{userLabel(u)}</span>
              {u.email && <span className="dialog__sub">{u.email}</span>}
            </button>
          ))}
          {open.length === 0 && busy && <div className="dialog__empty">Searching the directory…</div>}
          {open.length === 0 && !busy && <div className="dialog__empty">{failed ? "The directory did not answer. People from your spaces still show up here as you type." : `No one matching "${q.trim()}" in your spaces or the directory.`}</div>}
          {open.length > 0 && failed && <div className="picker__note">Directory unavailable, showing people from your spaces.</div>}
        </div>
      )}
    </div>
  );
}
