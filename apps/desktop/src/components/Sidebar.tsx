import { useLayoutEffect, useMemo, useRef, useState, type FormEvent, type ReactNode } from "react";
import type { Message, Space, User } from "../bridge-types";
import { addAccount, createSpace, markSpaceRead, openExternal, openFeed, openSpace, openThread, setPrefs, sidebarOrder, signOut, startConversation, switchAccount, toast, toggleMute, toggleStar, ui } from "../actions";
import { plainText } from "../lib/markdown";
import { stagger, useExit, useLast } from "../lib/motion";
import { relative, parseTime } from "../lib/time";
import { bodyOf, dmPeer, mentionsMe, nameMap, spaceLabel, todayKey, useStore } from "../store";
import { Avatar, Presence } from "./Avatar";
import { Icon } from "./icons";
import { Modal } from "./Modal";
import { PeoplePicker } from "./PeoplePicker";
import { liveHuddleIn } from "../lib/meet";
import { HuddleBar } from "./HuddleBar";
import { Menu, menuAt } from "./Rail";

const OPEN_KEY = "prism.sections";
function loadOpen(): Record<string, boolean> {
  try { return JSON.parse(localStorage.getItem(OPEN_KEY) ?? "{}") as Record<string, boolean>; } catch { return {}; }
}

export const isMeetChat = (s: Space): boolean => /\s-\s[A-Z][a-z]{2}\s\d{1,2}$/.test(s.displayName ?? "");

export function Sidebar() {
  const view = useStore((s) => s.ui.view);
  const spaces = useStore((s) => s.spaces);
  const starred = useStore((s) => s.starred);
  const browseOpen = useStore((s) => s.ui.browseOpen);
  const composeOpen = useStore((s) => s.ui.composeOpen);
  const titleMenu = useStore((s) => s.ui.titleMenu);
  const active = useStore((s) => (s.ui.feed ? `feed:${s.ui.feed}` : s.ui.activeSpace));
  const feed = useStore((s) => s.ui.feed);
  const unreadN = useStore((s) => Object.entries(s.unread).reduce((a, [sp, n]) => a + (s.muted.includes(sp) ? 0 : n), 0));
  const savedN = useStore((s) => s.saved.length);
  const todosN = useStore((s) => s.tasks.items.filter((t) => t.status !== "completed").length);
  const todayN = useStore((s) => { const k = todayKey(); return Object.values(s.calendar.events).filter((e) => e.status !== "cancelled" && (e.start.dateTime ?? e.start.date ?? "").startsWith(k) && !s.calendar.hidden.includes(e.calendarId ?? "primary")).length; });
  const [open, setOpen] = useState<Record<string, boolean>>(loadOpen);
  const toggle = (k: string) => setOpen((o) => {
    const n = { ...o, [k]: o[k] === false ? true : false };
    localStorage.setItem(OPEN_KEY, JSON.stringify(n));
    return n;
  });
  const named = spaces.filter((s) => (s.spaceType === "SPACE" || !s.spaceType) && !starred.includes(s.name));
  // note: Meet names its chat spaces "<meeting> - Mon D"; the API has no flag, so the name is the signal.
  const channels = named.filter((s) => !isMeetChat(s));
  const meet = named.filter(isMeetChat).sort((a, b) => (b.lastActiveTime ?? "").localeCompare(a.lastActiveTime ?? ""));
  const membersAll = useStore((s) => s.members);
  const meName = useStore((s) => s.auth.user?.name);
  const isApp = (s: Space) => dmPeer(s, membersAll[s.name], meName)?.type === "BOT";
  const dms = spaces.filter((s) => s.spaceType === "DIRECT_MESSAGE" && !starred.includes(s.name) && !isApp(s));
  const apps = spaces.filter((s) => s.spaceType === "DIRECT_MESSAGE" && !starred.includes(s.name) && isApp(s));
  const groups = spaces.filter((s) => s.spaceType === "GROUP_CHAT" && !starred.includes(s.name));
  const stars = spaces.filter((s) => starred.includes(s.name));
  const sorted = useMemo(() => sidebarOrder(spaces, membersAll, starred, meName), [spaces, membersAll, starred, meName]);
  const by = (list: Space[]) => sorted.filter((s) => list.includes(s));
  const pill = useGlidePill(active, [view, open, spaces.length, starred.length]);
  const browse = useExit(browseOpen, 140);
  const compose = useExit(composeOpen, 140);
  const menu = useExit(titleMenu, 120);

  return (
    <aside className="sidebar glass" aria-label="Spaces">
      <div className="sidebar__head">
        <button type="button" className="sidebar__title" aria-haspopup="menu" aria-expanded={titleMenu} onClick={() => ui({ titleMenu: !titleMenu })}>
          Confab <Icon n="down" />
        </button>
        <button type="button" className="iconbtn" aria-label="New message" title="New message (⌘N)" onClick={() => ui({ composeOpen: true })}>
          <Icon n="edit" />
        </button>
      </div>
      <div className="sidebar__scroll" ref={pill.scrollRef}>
        <div className={`sidebar__pill${pill.on ? " sidebar__pill--on" : ""}${pill.still ? " sidebar__pill--still" : ""}`} style={{ transform: `translateY(${pill.top}px)` }} aria-hidden="true" />
        <div className="sidebar__view" key={view}>
          {view === "activity" ? (
            <Activity />
          ) : (
            <>
              <div className="section section--feeds">
                <button type="button" className={`row${feed === "unreads" ? " row--active" : ""}${unreadN ? " row--unread" : ""}`} data-space="feed:unreads" aria-current={feed === "unreads" ? "page" : undefined} onClick={() => openFeed("unreads")}>
                  <span className="row__icon"><Icon n="inbox" /></span>
                  <span className="row__name">Unreads</span>
                  {unreadN > 0 && <span className="row__count">{unreadN}</span>}
                </button>
                <button type="button" className={`row${feed === "saved" ? " row--active" : ""}`} data-space="feed:saved" aria-current={feed === "saved" ? "page" : undefined} onClick={() => openFeed("saved")}>
                  <span className="row__icon"><Icon n="bookmark" /></span>
                  <span className="row__name">Saved</span>
                  {savedN > 0 && <span className="row__count">{savedN}</span>}
                </button>
                <button type="button" className={`row${feed === "todos" ? " row--active" : ""}`} data-space="feed:todos" aria-current={feed === "todos" ? "page" : undefined} onClick={() => openFeed("todos")}>
                  <span className="row__icon"><Icon n="check" /></span>
                  <span className="row__name">Todos</span>
                  {todosN > 0 && <span className="row__count">{todosN}</span>}
                </button>
                <button type="button" className={`row${feed === "people" ? " row--active" : ""}`} data-space="feed:people" aria-current={feed === "people" ? "page" : undefined} onClick={() => openFeed("people")}>
                  <span className="row__icon"><Icon n="users" /></span>
                  <span className="row__name">People</span>
                </button>
                <button type="button" className={`row${feed === "calendar" ? " row--active" : ""}`} data-space="feed:calendar" aria-current={feed === "calendar" ? "page" : undefined} onClick={() => openFeed("calendar")}>
                  <span className="row__icon"><Icon n="calendar" /></span>
                  <span className="row__name">Calendar</span>
                  {todayN > 0 && <span className="row__count">{todayN}</span>}
                </button>
                <button type="button" className={`row${feed === "notes" ? " row--active" : ""}`} data-space="feed:notes" aria-current={feed === "notes" ? "page" : undefined} onClick={() => openFeed("notes")}>
                  <span className="row__icon"><Icon n="edit" /></span>
                  <span className="row__name">Notes</span>
                </button>
              </div>
              {view === "home" && stars.length > 0 && (
                <Section id="starred" title="Starred" open={open.starred !== false} onToggle={toggle}>
                  {by(stars).map((s) => <SpaceRow key={s.name} space={s} />)}
                </Section>
              )}
              {view === "home" && (
                <Section id="channels" title="Spaces" open={open.channels !== false} onToggle={toggle} count={channels.length}>
                  {by(channels).map((s) => <SpaceRow key={s.name} space={s} />)}
                  <button type="button" className="row row--add" onClick={() => ui({ browseOpen: true })}>
                    <span className="row__icon">+</span>
                    <span className="row__name">Browse spaces</span>
                  </button>
                </Section>
              )}
              {view === "home" && meet.length > 0 && (
                <Section id="meet" title="Meeting chats" open={open.meet === true} onToggle={toggle} count={meet.length}>
                  {meet.map((s) => <SpaceRow key={s.name} space={s} />)}
                </Section>
              )}
              <Section id="dms" title="Direct messages" open={open.dms !== false} onToggle={toggle} count={dms.length}>
                {by(dms).map((s) => <SpaceRow key={s.name} space={s} />)}
                {dms.length === 0 && <div className="sidebar__empty">No direct messages yet.</div>}
              </Section>
              <Section id="groups" title="Group chats" open={open.groups !== false} onToggle={toggle} count={groups.length}>
                {by(groups).map((s) => <SpaceRow key={s.name} space={s} />)}
                {groups.length === 0 && <div className="sidebar__empty">No group chats yet.</div>}
              </Section>
              {apps.length > 0 && (
                <Section id="apps" title="Apps" open={open.apps !== false} onToggle={toggle} count={apps.length}>
                  {by(apps).map((s) => <SpaceRow key={s.name} space={s} />)}
                </Section>
              )}
            </>
          )}
        </div>
      </div>
      <HuddleBar />
      <Grip />
      {browse.mounted && <Browse exiting={browse.exiting} onClose={() => ui({ browseOpen: false })} />}
      {compose.mounted && <Compose exiting={compose.exiting} onClose={() => ui({ composeOpen: false })} />}
      {menu.mounted && <TitleMenu exiting={menu.exiting} onClose={() => ui({ titleMenu: false })} />}
    </aside>
  );
}

/**
 * One highlight that glides to the active row instead of each row painting its own.
 * Measured after layout; a section opening or closing re-measures when its transition settles.
 */
function useGlidePill(active: string | undefined, deps: unknown[]) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [state, setState] = useState({ top: 0, on: false, still: true });
  const seen = useRef(false);
  useLayoutEffect(() => {
    const measure = () => {
      const root = scrollRef.current;
      const el = active ? root?.querySelector<HTMLElement>(`[data-space="${CSS.escape(active)}"]`) : null;
      if (!root || !el || el.offsetHeight === 0) { setState((s) => ({ ...s, on: false })); return; }
      const top = el.getBoundingClientRect().top - root.getBoundingClientRect().top + root.scrollTop;
      // The first placement, and any placement after being hidden, jumps rather than glides.
      const still = !seen.current;
      seen.current = true;
      setState({ top, on: true, still });
    };
    measure();
    const t = window.setTimeout(measure, 340);
    return () => window.clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, ...deps]);
  // After a hidden spell the next show should jump, not glide from a stale place.
  useLayoutEffect(() => { if (!state.on) seen.current = false; }, [state.on]);
  return { scrollRef, ...state };
}

function Section({ id, title, open, onToggle, count, children }: { id: string; title: string; open: boolean; onToggle: (k: string) => void; count?: number; children: ReactNode }) {
  return (
    <div className={`section${open ? "" : " section--closed"}`}>
      <button type="button" className="section__head" aria-expanded={open} onClick={() => onToggle(id)}>
        <Icon n="down" />
        {title}
        {!open && count !== undefined && <span className="section__count">{count}</span>}
      </button>
      <div className="section__body">
        <div className="section__inner" inert={!open}>{children}</div>
      </div>
    </div>
  );
}

function SpaceRow({ space }: { space: Space }) {
  const active = useStore((s) => s.ui.activeSpace === space.name && !s.ui.feed);
  const unread = useStore((s) => s.unread[space.name] ?? 0);
  const muted = useStore((s) => s.muted.includes(space.name));
  const starred = useStore((s) => s.starred.includes(space.name));
  const draft = useStore((s) => !!s.drafts[space.name]?.trim() && s.ui.activeSpace !== space.name);
  const voiceN = useStore((s) => (s.huddles[space.name] ?? []).length);
  // A string, not an object: a selector that returns a fresh object every call would loop the store.
  const liveName = useStore((s) => liveHuddleIn(s.messages[space.name]?.items)?.message.name ?? "");
  const live = voiceN || (liveName ? 1 : 0);
  const members = useStore((s) => s.members[space.name]);
  const me = useStore((s) => s.auth.user?.name);
  const peer = dmPeer(space, members, me);
  const presence = useStore((s) => (peer ? s.presence[peer.name]?.state : undefined));
  const label = spaceLabel(space, members, me);
  const [ctx, setCtx] = useState<React.CSSProperties | null>(null);
  const menu = useExit(!!ctx, 120);
  const style = useLast(ctx);
  const link = `https://mail.google.com/chat/u/0/#chat/space/${space.name.split("/")[1]}`;
  return (
    <>
      <button
        type="button"
        className={`row${active ? " row--active" : ""}${unread && !muted ? " row--unread" : ""}${muted ? " row--muted" : ""}${ctx ? " row--menu" : ""}`}
        aria-current={active ? "page" : undefined}
        data-space={space.name}
        onClick={() => openSpace(space.name)}
        onContextMenu={(e) => { e.preventDefault(); setCtx(menuAt(e, 232, 260)); }}
      >
        <span className="row__icon">
          {space.spaceType === "DIRECT_MESSAGE" ? <Avatar user={peer} size={20} /> : space.spaceType === "GROUP_CHAT" ? <Icon n="users" /> : isMeetChat(space) ? <Icon n="calendar" /> : "#"}
        </span>
        {space.spaceType === "DIRECT_MESSAGE" && <Presence state={presence ?? "offline"} />}
        <span className="row__name">{label}</span>
        {live > 0 && <span className="row__live" title={liveName ? "A huddle is on" : `${voiceN} in a voice huddle`}><Icon n="headphones" />{voiceN || ""}</span>}
        {draft && <span className="row__draft" title="Unsent draft"><Icon n="edit" /></span>}
        {muted ? <span className="row__mute" title="Muted"><Icon n="belloff" /></span> : unread > 0 && <span key={unread} className="row__badge" aria-label={`${unread} unread`}>{unread > 99 ? "99+" : unread}</span>}
      </button>
      {menu.mounted && style && (
        <Menu className="menu--ctx" label={label} style={style} exiting={menu.exiting} onClose={() => setCtx(null)}>
          <button type="button" role="menuitem" className="menu__item" onClick={() => { setCtx(null); toggleStar(space.name); }}><Icon n="star" fill={starred} />{starred ? "Unstar" : "Star"}</button>
          <button type="button" role="menuitem" className="menu__item" onClick={() => { setCtx(null); toggleMute(space.name); }}><Icon n={muted ? "bell" : "belloff"} />{muted ? "Unmute" : "Mute"}</button>
          <button type="button" role="menuitem" className="menu__item" disabled={!unread} onClick={() => { setCtx(null); markSpaceRead(space.name); }}><Icon n="check" />Mark as read</button>
          <div className="menu__sep" />
          <button type="button" role="menuitem" className="menu__item" onClick={() => { setCtx(null); void navigator.clipboard?.writeText(link); toast("Link copied"); }}><Icon n="link" />Copy link</button>
          <button type="button" role="menuitem" className="menu__item" onClick={() => { setCtx(null); void openExternal(link); }}><Icon n="external" />Open in Google Chat</button>
        </Menu>
      )}
    </>
  );
}

/** The sidebar's right edge drags to resize it; double-click puts it back. */
function Grip() {
  const start = useRef<{ x: number; w: number } | null>(null);
  const onDown = (e: React.PointerEvent<HTMLDivElement>) => {
    start.current = { x: e.clientX, w: useStore.getState().prefs.sidebarW };
    e.currentTarget.setPointerCapture(e.pointerId);
  };
  const onMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (!start.current) return;
    const w = Math.round(Math.min(420, Math.max(200, start.current.w + e.clientX - start.current.x)));
    if (w !== useStore.getState().prefs.sidebarW) setPrefs({ sidebarW: w });
  };
  const onUp = () => { start.current = null; };
  return <div className="sidebar__grip" role="separator" aria-orientation="vertical" aria-label="Resize sidebar" onPointerDown={onDown} onPointerMove={onMove} onPointerUp={onUp} onPointerCancel={onUp} onDoubleClick={() => setPrefs({ sidebarW: 260 })} />;
}

/** Mentions of me across loaded messages. note: only what is in memory, no server-side activity feed. */
function Activity() {
  const me = useStore((s) => s.auth.user?.name);
  const messages = useStore((s) => s.messages);
  const members = useStore((s) => s.members);
  const spaces = useStore((s) => s.spaces);
  const hits = useMemo(() => {
    if (!me) return [] as Message[];
    const out: Message[] = [];
    for (const page of Object.values(messages)) for (const m of page.items) if (mentionsMe(m, me)) out.push(m);
    return out.sort((a, b) => ((a.createTime ?? "") < (b.createTime ?? "") ? 1 : -1)).slice(0, 50);
  }, [messages, me]);
  if (!hits.length) return <div className="sidebar__empty">Mentions of you show up here as they arrive.</div>;
  return (
    <>
      {hits.map((m, i) => {
        const space = m.name.split("/").slice(0, 2).join("/");
        const sp = spaces.find((s) => s.name === space);
        const names = nameMap(members[space]);
        return (
          <button
            key={m.name}
            type="button"
            className="activity"
            style={stagger(i)}
            onClick={() => {
              openSpace(space);
              if (m.threadReply && m.thread?.name) openThread(m.thread.name);
            }}
          >
            <Avatar user={m.sender} size={28} />
            <div>
              <div className="activity__meta">
                <b>{m.sender?.displayName ?? "Someone"}</b>
                <span>in {sp ? spaceLabel(sp, members[space], me) : space}</span>
                <span>{relative(parseTime(m.createTime))}</span>
              </div>
              <div className="activity__text">{plainText(bodyOf(m), (u) => names[u]?.displayName ?? u)}</div>
            </div>
          </button>
        );
      })}
    </>
  );
}

function TitleMenu({ onClose, exiting }: { onClose: () => void; exiting?: boolean }) {
  const user = useStore((s) => s.auth.user);
  const accounts = useStore((s) => s.accounts);
  const shell = useStore((s) => s.shell);
  return (
    <Menu className="menu--title" label="Workspace" onClose={onClose} exiting={exiting}>
      <div className="menu__who">
        <Avatar user={user} size={36} />
        <div>
          <b>Confab</b>
          <span>{user?.email ?? user?.displayName ?? ""}</span>
        </div>
      </div>
      <button type="button" role="menuitem" className="menu__item" onClick={() => { onClose(); ui({ composeOpen: true }); }}>
        <Icon n="edit" />New message<span className="kbd">⌘N</span>
      </button>
      <button type="button" role="menuitem" className="menu__item" onClick={() => { onClose(); ui({ browseOpen: true }); }}>
        <Icon n="hash" />Browse spaces
      </button>
      <button type="button" role="menuitem" className="menu__item" onClick={() => { onClose(); ui({ switcher: true }); }}>
        <Icon n="search" />Jump to<span className="kbd">⌘K</span>
      </button>
      <button type="button" role="menuitem" className="menu__item" onClick={() => { onClose(); ui({ shortcutsOpen: true }); }}>
        <Icon n="keyboard" />Keyboard shortcuts<span className="kbd">⌘/</span>
      </button>
      <div className="menu__sep" />
      <button type="button" role="menuitem" className="menu__item" onClick={() => { onClose(); void openExternal("https://chat.google.com/"); }}>
        <Icon n="external" />Open Google Chat on the web
      </button>
      <button type="button" role="menuitem" className="menu__item" onClick={() => { onClose(); ui({ prefsOpen: true }); }}>
        <Icon n="sliders" />Preferences
      </button>
      {shell !== "browser" && (
        <>
          <div className="menu__sep" />
          {accounts.filter((a) => !a.active && a.signedIn).map((a) => (
            <button key={a.id} type="button" role="menuitem" className="menu__item" onClick={() => { onClose(); void switchAccount(a.id); }}>
              <Avatar user={{ name: a.id, displayName: a.displayName ?? a.email ?? "?", avatarUrl: a.photo }} size={18} />{a.displayName ?? a.email ?? a.id}{a.unread > 0 && <span className="kbd">{a.unread}</span>}
            </button>
          ))}
          <button type="button" role="menuitem" className="menu__item" onClick={() => { onClose(); void addAccount(); }}>
            <Icon n="plus" />Add another Google account
          </button>
        </>
      )}
      <div className="menu__sep" />
      <button type="button" role="menuitem" className="menu__item" onClick={() => { onClose(); void signOut(); }}>
        <Icon n="logout" />Sign out
      </button>
    </Menu>
  );
}

/** Slack's compose: pick people, one opens a DM, more open a group chat. */
function Compose({ onClose, exiting }: { onClose: () => void; exiting?: boolean }) {
  const [people, setPeople] = useState<User[]>([]);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const go = (e: FormEvent) => {
    e.preventDefault();
    if (!people.length || busy) return;
    setBusy(true);
    setErr("");
    startConversation(people).then(onClose).catch((x: unknown) => { setErr((x as Error)?.message ?? String(x)); setBusy(false); });
  };
  return (
    <Modal title="New message" onClose={onClose} exiting={exiting}>
      <form onSubmit={go}>
        <div className="dialog__body">
          <div className="field">
            <span>To</span>
            <PeoplePicker value={people} onChange={setPeople} autoFocus />
          </div>
          <div className="dialog__note">{people.length <= 1 ? "One person starts a direct message." : `${people.length} people start a group chat.`}</div>
          {err && <div className="dialog__note dialog__note--err" role="alert">{err}</div>}
        </div>
        <div className="dialog__actions">
          <button type="button" className="btn" onClick={onClose} disabled={busy}>Cancel</button>
          <button type="submit" className="btn btn--primary" disabled={busy || !people.length}>{busy ? "Opening…" : people.length > 1 ? "Start group chat" : "Message"}</button>
        </div>
      </form>
    </Modal>
  );
}

function Browse({ onClose, exiting }: { onClose: () => void; exiting?: boolean }) {
  const spaces = useStore((s) => s.spaces);
  const members = useStore((s) => s.members);
  const me = useStore((s) => s.auth.user?.name);
  const [q, setQ] = useState("");
  const [creating, setCreating] = useState(false);
  const named = spaces.filter((s) => s.spaceType === "SPACE" || !s.spaceType);
  const list = [...named.filter((s) => !isMeetChat(s)), ...named.filter(isMeetChat)]
    .filter((s) => !q || spaceLabel(s, members[s.name], me).toLowerCase().includes(q.toLowerCase()) || (s.spaceDetails?.description ?? "").toLowerCase().includes(q.toLowerCase()));
  if (creating) return <CreateSpace onClose={onClose} onBack={() => setCreating(false)} exiting={exiting} />;
  return (
    <Modal title="Browse spaces" onClose={onClose} exiting={exiting}>
      <input className="dialog__input" autoFocus value={q} onChange={(e) => setQ(e.target.value)} placeholder="Filter spaces" aria-label="Filter spaces" />
      <div className="dialog__list">
        {list.map((s) => (
          <button key={s.name} type="button" className="dialog__item" onClick={() => openSpace(s.name)}>
            <span className="row__icon">{isMeetChat(s) ? <Icon n="calendar" /> : "#"}</span>
            <span>
              <b>{spaceLabel(s, members[s.name], me)}</b>
              {s.spaceDetails?.description && <span className="dialog__kind" style={{ marginLeft: 8 }}>{s.spaceDetails.description}</span>}
            </span>
            <span className="dialog__kind">{s.membershipCount?.joinedDirectHumanUserCount ?? ""}{s.membershipCount?.joinedDirectHumanUserCount ? " members" : ""}</span>
          </button>
        ))}
        {list.length === 0 && <div className="dialog__empty">No space matches "{q}". Spaces you are not in do not appear here yet.</div>}
      </div>
      <div className="dialog__actions">
        <button type="button" className="btn" onClick={() => setCreating(true)}><Icon n="plus" />Create a space</button>
      </div>
    </Modal>
  );
}

function CreateSpace({ onClose, onBack, exiting }: { onClose: () => void; onBack: () => void; exiting?: boolean }) {
  const [name, setName] = useState("");
  const [people, setPeople] = useState<User[]>([]);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!name.trim() || busy) return;
    setBusy(true);
    setErr("");
    createSpace(name.trim(), people).then(onClose).catch((x: unknown) => { setErr((x as Error)?.message ?? String(x)); setBusy(false); });
  };
  return (
    <Modal title="Create a space" onClose={onClose} exiting={exiting}>
      <form onSubmit={submit}>
        <div className="dialog__body">
          <label className="field">
            <span>Name</span>
            <input autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. launch-week" required maxLength={128} />
          </label>
          <div className="field">
            <span>People</span>
            <PeoplePicker value={people} onChange={setPeople} />
          </div>
          {err && <div className="dialog__note dialog__note--err" role="alert">{err}</div>}
        </div>
        <div className="dialog__actions">
          <button type="button" className="btn" onClick={onBack} disabled={busy}>Back</button>
          <button type="submit" className="btn btn--primary" disabled={busy || !name.trim()}>{busy ? "Creating…" : "Create"}</button>
        </div>
      </form>
    </Modal>
  );
}
