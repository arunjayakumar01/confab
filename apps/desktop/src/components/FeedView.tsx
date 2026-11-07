import { useEffect, useMemo } from "react";
import type { Message, Space } from "../bridge-types";
import { closeFeed, ensureMessages, jumpTo, markAllRead, markSpaceRead, openSpace, sidebarOrder, toggleSaved } from "../actions";
import { stagger } from "../lib/motion";
import { dateShort, parseTime } from "../lib/time";
import { bodyOf, nameMap, spaceLabel, useStore, type Feed, type SavedItem } from "../store";
import { Avatar } from "./Avatar";
import { Icon } from "./icons";
import { MarkdownView } from "./MarkdownView";
import { positions } from "./MessageList";
import { MessageRow } from "./MessageRow";
import { CalendarView } from "./CalendarView";
import { NotesView } from "./NotesView";
import { People, Todos } from "./Todos";

/** Unreads, Saved, Todos and People in the main pane, like Slack's views: one header, then the list. */
export function FeedView({ feed }: { feed: Feed }) {
  if (feed === "todos") return <Todos />;
  if (feed === "people") return <People />;
  if (feed === "calendar") return <CalendarView />;
  if (feed === "notes") return <NotesView />;
  return feed === "unreads" ? <Unreads /> : <Saved />;
}

function FeedHeader({ title, sub, action }: { title: string; sub: string; action?: React.ReactNode }) {
  return (
    <div className="header glass-strong">
      <div className="header__name"><span key={title} className="header__name-in">{title}</span></div>
      <div className="header__meta"><span className="header__desc">{sub}</span></div>
      <div className="header__spacer" />
      {action}
      <button type="button" className="iconbtn" aria-label="Close" onClick={closeFeed}><Icon n="x" /></button>
    </div>
  );
}

function Unreads() {
  const spaces = useStore((s) => s.spaces);
  const unread = useStore((s) => s.unread);
  const muted = useStore((s) => s.muted);
  const members = useStore((s) => s.members);
  const messages = useStore((s) => s.messages);
  const starred = useStore((s) => s.starred);
  const me = useStore((s) => s.auth.user?.name);
  const groups = useMemo(() => {
    const order = sidebarOrder(spaces, members, starred, me);
    return order
      .filter((sp) => (unread[sp.name] ?? 0) > 0 && !muted.includes(sp.name))
      .map((sp) => {
        const n = unread[sp.name]!;
        const top = (messages[sp.name]?.items ?? []).filter((m) => !m.threadReply);
        return { sp, n, items: top.slice(-n), loaded: !!messages[sp.name]?.loaded };
      });
  }, [spaces, members, starred, me, unread, muted, messages]);
  useEffect(() => { for (const g of groups) if (!g.loaded) void ensureMessages(g.sp.name); }, [groups]);
  const total = groups.reduce((a, g) => a + g.n, 0);
  return (
    <>
      <FeedHeader
        title="Unreads"
        sub={total ? `${total} unread in ${groups.length} ${groups.length === 1 ? "space" : "spaces"}` : "Nothing unread"}
        action={total > 0 ? <button type="button" className="btn feed__btn" onClick={markAllRead}><Icon n="checkall" />Mark all as read</button> : undefined}
      />
      <div className="feed" role="list" aria-label="Unread messages">
        {groups.length === 0 && (
          <div className="feed__empty">
            <Icon n="check" size={28} />
            <b>You're all caught up</b>
            New messages in spaces you have not opened collect here.
          </div>
        )}
        {groups.map((g, i) => (
          <section key={g.sp.name} className="feed__group" style={stagger(i)}>
            <GroupHead sp={g.sp} n={g.n} />
            {g.items.length === 0 && <div className="list__top">{g.loaded ? "Older than what is loaded; open the space to read it." : "Loading…"}</div>}
            {positions(g.items).map((p, j) => {
              const m = g.items[j]!;
              return <MessageRow key={m.name} m={m} collapsed={p === "mid" || p === "last"} pos={p} me={me} members={nameMap(members[g.sp.name])} inThread />;
            })}
          </section>
        ))}
      </div>
    </>
  );
}

function GroupHead({ sp, n }: { sp: Space; n?: number }) {
  const members = useStore((s) => s.members[sp.name]);
  const me = useStore((s) => s.auth.user?.name);
  const label = spaceLabel(sp, members, me);
  return (
    <div className="feed__head">
      <button type="button" className="feed__space" onClick={() => openSpace(sp.name)}>
        <span className="row__icon">{sp.spaceType === "SPACE" ? "#" : <Icon n={sp.spaceType === "GROUP_CHAT" ? "users" : "dms"} />}</span>
        {label}
        {n !== undefined && <span className="row__badge">{n}</span>}
      </button>
      <span className="feed__line" />
      {n !== undefined && <button type="button" className="feed__mark" onClick={() => markSpaceRead(sp.name)}>Mark as read</button>}
    </div>
  );
}

function Saved() {
  const saved = useStore((s) => s.saved);
  return (
    <>
      <FeedHeader title="Saved" sub={saved.length ? `${saved.length} ${saved.length === 1 ? "message" : "messages"} saved for later` : "Nothing saved yet"} />
      <div className="feed" role="list" aria-label="Saved messages">
        {saved.length === 0 && (
          <div className="feed__empty">
            <Icon n="bookmark" size={28} />
            <b>Save messages for later</b>
            The bookmark on any message keeps it here until you are done with it.
          </div>
        )}
        {saved.map((it, i) => <SavedRow key={it.name} it={it} i={i} />)}
      </div>
    </>
  );
}

function SavedRow({ it, i }: { it: SavedItem; i: number }) {
  const sp = useStore((s) => s.spaces.find((x) => x.name === it.space));
  const members = useStore((s) => s.members[it.space]);
  const me = useStore((s) => s.auth.user?.name);
  const live = useStore((s) => s.messages[it.space]?.items.find((m) => m.name === it.name));
  const m: Message = live ?? { name: it.name, text: it.text, sender: it.sender, createTime: it.createTime };
  const names = nameMap(members);
  const label = sp ? spaceLabel(sp, members, me) : it.space;
  return (
    <div className="saved" role="listitem" style={stagger(i)}>
      <div className="saved__meta">
        <Avatar user={m.sender} size={20} />
        <b>{m.sender?.displayName ?? "Someone"}</b>
        <span className="result__space">{sp?.spaceType === "SPACE" ? `#${label}` : label}</span>
        <span>{dateShort(parseTime(m.createTime))}</span>
        {!live && <span className="saved__gone">not loaded</span>}
      </div>
      {bodyOf(m) ? <MarkdownView text={bodyOf(m)} members={names} me={me} /> : <div className="msg__text"><em>Attachment only</em></div>}
      <div className="saved__actions">
        <button type="button" className="btn btn--primary" onClick={() => jumpTo(it.name)}>Jump to message</button>
        <button type="button" className="btn" onClick={() => toggleSaved(m)}>Remove</button>
      </div>
    </div>
  );
}
