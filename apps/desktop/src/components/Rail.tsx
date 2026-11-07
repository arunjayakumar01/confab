import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { PresenceState } from "../bridge-types";
import { addAccount, removeAccount, setPresence, signOut, switchAccount, ui } from "../actions";

/** Where a context menu opens for a pointer event: at the pointer, flipped up or left when it would run off the window. */
export function menuAt(e: { clientX: number; clientY: number }, w = 232, h = 340): React.CSSProperties {
  const left = Math.min(e.clientX, window.innerWidth - w - 8);
  const up = e.clientY + h > window.innerHeight - 8;
  return up ? { left, bottom: window.innerHeight - e.clientY, transformOrigin: "bottom left" } : { left, top: e.clientY, transformOrigin: "top left" };
}
import { useExit, useFlash, useLast } from "../lib/motion";
import { useStore, type RailView } from "../store";
import { Avatar, Presence } from "./Avatar";
import { Icon, ConfabMark, type IconName } from "./icons";

const VIEWS: { id: RailView; label: string; icon: IconName }[] = [
  { id: "home", label: "Home", icon: "home" },
  { id: "dms", label: "DMs", icon: "dms" },
  { id: "activity", label: "Activity", icon: "bell" },
];

export function Rail() {
  const view = useStore((s) => s.ui.view);
  const user = useStore((s) => s.auth.user);
  const me = useStore((s) => s.me);
  const open = useStore((s) => s.ui.statusMenu);
  const unread = useStore((s) => s.unread);
  const muted = useStore((s) => s.muted);
  const spaces = useStore((s) => s.spaces);
  const relay = useStore((s) => s.relay);
  const live = (sp: { name: string }) => !muted.includes(sp.name) && (unread[sp.name] ?? 0) > 0;
  const anyUnread = spaces.some(live);
  const dmUnread = spaces.some((sp) => sp.spaceType !== "SPACE" && live(sp));
  // The mark glows once each time realtime comes (back) up.
  const flash = useFlash(relay === "open" ? "open" : "down", 900) && relay === "open";
  const menu = useExit(open, 120);

  return (
    <nav className="rail" aria-label="Workspace">
      <div className={`rail__mark${flash ? " rail__mark--flash" : ""}`} title={relay === "open" ? "Confab, realtime connected" : "Confab"}>
        <ConfabMark />
      </div>
      <Accounts />
      {VIEWS.map((v) => (
        <button
          key={v.id}
          type="button"
          className={`rail__btn${view === v.id ? " rail__btn--on" : ""}`}
          aria-pressed={view === v.id}
          onClick={() => ui({ view: v.id })}
        >
          <Icon n={v.icon} />
          {v.label}
          {((v.id === "home" && anyUnread) || (v.id === "dms" && dmUnread)) && <span className="rail__dot" aria-label="Unread" />}
        </button>
      ))}
      <button type="button" className="rail__btn" onClick={() => ui({ prefsOpen: true })}>
        <Icon n="more" />
        More
      </button>
      <div className="rail__spacer" />
      <button
        type="button"
        className="rail__me"
        aria-label="Your status"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => ui({ statusMenu: !open })}
      >
        <Avatar user={user} size={36} />
        <Presence state={me.state} />
      </button>
      {menu.mounted && <StatusMenu exiting={menu.exiting} onClose={() => ui({ statusMenu: false })} />}
    </nav>
  );
}

/** The account column: one tile per Google account, the active one framed, unread from the others, and a plus. */
function Accounts() {
  const accounts = useStore((s) => s.accounts);
  const shell = useStore((s) => s.shell);
  const [ctx, setCtx] = useState<{ id: string; style: React.CSSProperties } | null>(null);
  const menu = useExit(!!ctx, 120);
  const last = useLast(ctx);
  if (shell === "browser" || accounts.length === 0) return null;
  return (
    <div className="rail__accounts" role="group" aria-label="Accounts">
      {accounts.map((a) => (
        <button
          key={a.id}
          type="button"
          className={`rail__acct${a.active ? " rail__acct--on" : ""}${a.signedIn ? "" : " rail__acct--out"}`}
          aria-pressed={a.active}
          title={a.signedIn ? `${a.displayName ?? a.email ?? a.id}${a.unread ? `, ${a.unread} unread` : ""}` : "Signing in…"}
          onClick={() => void switchAccount(a.id)}
          onContextMenu={(e) => { e.preventDefault(); setCtx({ id: a.id, style: menuAt(e, 220, 120) }); }}
        >
          <Avatar user={{ name: a.id, displayName: a.displayName ?? a.email ?? "?", avatarUrl: a.photo }} size={26} />
          {a.unread > 0 && !a.active && <span className="rail__acctbadge">{a.unread > 9 ? "9+" : a.unread}</span>}
        </button>
      ))}
      <button type="button" className="rail__acct rail__acct--add" title="Add another Google account" aria-label="Add another Google account" onClick={() => void addAccount()}><Icon n="plus" size={14} /></button>
      {menu.mounted && last && (
        <Menu className="menu--ctx" label="Account" style={last.style} exiting={menu.exiting} onClose={() => setCtx(null)}>
          <div className="menu__note">{accounts.find((a) => a.id === last.id)?.email ?? last.id}</div>
          <button type="button" role="menuitem" className="menu__item" onClick={() => { setCtx(null); void switchAccount(last.id); }}><Icon n="check" />Switch to this account</button>
          <button type="button" role="menuitem" className="menu__item menu__item--danger" disabled={accounts.find((a) => a.id === last.id)?.active} onClick={() => { setCtx(null); if (window.confirm("Remove this account from Confab? Its sign-in and cache on this Mac go; nothing changes at Google.")) void removeAccount(last.id); }}><Icon n="logout" />Remove from Confab</button>
        </Menu>
      )}
    </div>
  );
}

/** A floating menu portalled to body: outside click or Esc call `onClose`; the host passes `exiting` for the close animation. */
export function Menu({ className, label, onClose, exiting, style, children }: { className: string; label: string; onClose: () => void; exiting?: boolean; style?: React.CSSProperties; children: React.ReactNode }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [onClose]);
  return createPortal(
    <div ref={ref} className={`menu ${className}${exiting ? " menu--out" : ""}`} role="menu" aria-label={label} style={style}>
      {children}
    </div>,
    document.body,
  );
}

function StatusMenu({ onClose, exiting }: { onClose: () => void; exiting?: boolean }) {
  const user = useStore((s) => s.auth.user);
  const me = useStore((s) => s.me);
  const [emoji, setEmoji] = useState(me.custom?.emoji ?? "");
  const [text, setText] = useState(me.custom?.text ?? "");
  const [clearAfter, setClearAfter] = useState<number | undefined>(undefined);
  const [dndOpen, setDndOpen] = useState(false);
  const pick = (state: PresenceState, dndSeconds?: number) => {
    setPresence(state, me.custom, { dndSeconds });
    onClose();
  };
  const saveCustom = () => {
    const t = text.trim().slice(0, 64);
    setPresence(me.state, t ? { emoji: emoji.trim() || undefined, text: t } : undefined, { statusSeconds: clearAfter, pushStatus: true });
    onClose();
  };
  const DND: [string, number][] = [["30 minutes", 1800], ["1 hour", 3600], ["2 hours", 7200], ["4 hours", 14400], ["Until tomorrow", Math.max(60, Math.round((new Date(new Date().setHours(33, 0, 0, 0)).getTime() - Date.now()) / 1000))]];
  const items: { s: PresenceState; label: string; icon: IconName }[] = [
    { s: "active", label: "Active", icon: "check" },
    { s: "away", label: "Away", icon: "clock" },
    { s: "dnd", label: "Do not disturb", icon: "minus" },
  ];
  return (
    <Menu className="menu--rail" label="Status" onClose={onClose} exiting={exiting}>
      <div className="menu__who">
        <Avatar user={user} size={36} />
        <div>
          <b>{user?.displayName ?? "You"}</b>
          <span>{me.custom ? `${me.custom.emoji ?? ""} ${me.custom.text}`.trim() : user?.email ?? ""}</span>
        </div>
      </div>
      {items.map((it) => (
        <button key={it.s} type="button" role="menuitemradio" aria-checked={me.state === it.s} className={`menu__item${me.state === it.s ? " menu__item--on" : ""}`} onClick={() => (it.s === "dnd" ? setDndOpen((o) => !o) : pick(it.s))} aria-haspopup={it.s === "dnd" ? "menu" : undefined} aria-expanded={it.s === "dnd" ? dndOpen : undefined}>
          <Presence state={it.s} />
          {it.label}
          {it.s === "dnd" && <Icon n={dndOpen ? "down" : "right"} size={12} />}
        </button>
      ))}
      {dndOpen && (
        <div className="menu__sub" role="group" aria-label="Do not disturb for">
          {DND.map(([label, secs]) => <button key={label} type="button" role="menuitem" className="menu__item menu__item--sub" onClick={() => pick("dnd", secs)}>{label}</button>)}
        </div>
      )}
      <div className="menu__sep" />
      <form
        className="menu__status"
        onSubmit={(e) => {
          e.preventDefault();
          saveCustom();
        }}
      >
        <input className="emoji-in" value={emoji} onChange={(e) => setEmoji(e.target.value)} placeholder="🙂" aria-label="Status emoji" maxLength={4} />
        <input value={text} onChange={(e) => setText(e.target.value)} placeholder="What's your status?" aria-label="Status text" maxLength={64} />
        <button type="submit" className="iconbtn" aria-label="Save status">
          <Icon n="check" />
        </button>
      </form>
      <div className="menu__status menu__status--opts">
        <select value={clearAfter ?? ""} onChange={(e) => setClearAfter(e.target.value ? Number(e.target.value) : undefined)} aria-label="Clear the status after">
          <option value="">Keep until cleared</option><option value="1800">Clear in 30 minutes</option><option value="3600">Clear in 1 hour</option><option value="14400">Clear in 4 hours</option><option value="86400">Clear tomorrow</option>
        </select>
        {me.custom && <button type="button" className="btn" onClick={() => { setText(""); setEmoji(""); setPresence(me.state, undefined, { pushStatus: true }); onClose(); }}>Clear</button>}
      </div>
      <div className="menu__note">Also set in Google Chat and Gmail. People on Confab see it at once; Google shows it everywhere else.</div>
      <div className="menu__sep" />
      <button type="button" role="menuitem" className="menu__item" onClick={() => { onClose(); ui({ prefsOpen: true }); }}>
        <Icon n="sliders" size={16} />
        Preferences
      </button>
      <button type="button" role="menuitem" className="menu__item" onClick={() => void signOut()}>
        <Icon n="logout" size={16} />
        Sign out
      </button>
    </Menu>
  );
}
