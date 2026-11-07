import { useMemo, useState, type FormEvent } from "react";
import type { Task, User } from "../bridge-types";
import { addTask, closeFeed, loadTasks, openDm, openExternal, removeTask, toggleTask, ui } from "../actions";
import { stagger } from "../lib/motion";
import { dateShort, parseTime } from "../lib/time";
import { spaceLabel, useStore } from "../store";
import { Avatar, Presence } from "./Avatar";
import { Icon } from "./icons";
import { usePeopleSearch, userLabel } from "./PeoplePicker";

function Head({ title, sub, action }: { title: string; sub: string; action?: React.ReactNode }) {
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

/** Google Tasks, default list: add, tick, delete; "Add to Tasks" on a message lands here too. */
export function Todos() {
  const { items: all, loaded, error } = useStore((s) => s.tasks);
  const todosSpace = useStore((s) => s.ui.todosSpace);
  const sp = useStore((s) => s.spaces.find((x) => x.name === s.ui.todosSpace));
  const spMembers = useStore((s) => (s.ui.todosSpace ? s.members[s.ui.todosSpace] : undefined));
  const me = useStore((s) => s.auth.user?.name);
  const [title, setTitle] = useState("");
  // Tasks made from a space carry its message link in their notes; that is the whole filter.
  const items = todosSpace ? all.filter((t) => t.notes?.includes(`#chat/space/${todosSpace.split("/")[1]}/`)) : all;
  const open = items.filter((t) => t.status !== "completed");
  const done = items.filter((t) => t.status === "completed");
  const spLabel = sp ? spaceLabel(sp, spMembers, me) : "";
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!title.trim()) return;
    void addTask(title);
    setTitle("");
  };
  return (
    <>
      <Head
        title="Todos"
        sub={!loaded ? "Loading Google Tasks…" : error ? "Google Tasks did not answer" : `${open.length} open${done.length ? `, ${done.length} done` : ""}`}
        action={<button type="button" className="btn feed__btn" onClick={() => void openExternal("https://tasks.google.com/")}><Icon n="external" />Open Google Tasks</button>}
      />
      <div className="feed" role="list" aria-label="Tasks">
        {todosSpace && (
          <div className="todo__filter">
            <span className="chip">From {sp?.spaceType === "SPACE" ? `#${spLabel}` : spLabel}<button type="button" aria-label="Show all tasks" onClick={() => ui({ todosSpace: undefined })}><Icon n="x" size={12} /></button></span>
            <span>Tasks you added from this space. Right-click a message there and pick Add to Tasks.</span>
          </div>
        )}
        <form className="todo__add" onSubmit={submit}>
          <Icon n="plus" />
          <input value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Add a task to Google Tasks" aria-label="New task" maxLength={1024} />
          <button type="submit" className="btn btn--primary" disabled={!title.trim()}>Add</button>
        </form>
        {error && (
          <div className="feed__empty">
            <Icon n="x" size={28} />
            <b>Google Tasks did not answer</b>
            {error.includes("403") || error.includes("insufficient") || error.includes("SERVICE_DISABLED") ? "Tasks needs the Tasks API enabled in the GCP project and a fresh sign-in for the new scope." : error}
            <button type="button" className="btn" onClick={() => void loadTasks()}>Try again</button>
          </div>
        )}
        {loaded && !error && items.length === 0 && (
          <div className="feed__empty">
            <Icon n="check" size={28} />
            <b>Nothing to do</b>
            Add one above, or right-click a message and pick Add to Tasks.
          </div>
        )}
        {open.map((t, i) => <TaskRow key={t.id} t={t} i={i} />)}
        {done.length > 0 && <div className="feed__head"><span className="feed__done">Done</span><span className="feed__line" /></div>}
        {done.map((t, i) => <TaskRow key={t.id} t={t} i={i} />)}
      </div>
    </>
  );
}

function TaskRow({ t, i }: { t: Task; i: number }) {
  const done = t.status === "completed";
  const due = t.due ? parseTime(t.due) : null;
  const overdue = !!due && !done && due.getTime() < Date.now() - 86_400_000;
  const link = t.notes?.match(/https?:\/\/\S+/)?.[0];
  return (
    <div className={`todo${done ? " todo--done" : ""}`} role="listitem" style={stagger(i)}>
      <button type="button" className="todo__check" role="checkbox" aria-checked={done} aria-label={done ? "Reopen" : "Mark done"} onClick={() => void toggleTask(t)}>
        <Icon n="check" size={12} />
      </button>
      <div className="todo__body">
        <div className="todo__title">{t.title}</div>
        {(t.notes || due) && (
          <div className="todo__meta">
            {due && <span className={`todo__due${overdue ? " todo__due--late" : ""}`}><Icon n="calendar" size={12} />{dateShort(due)}</span>}
            {t.notes && <span className="todo__notes">{t.notes.replace(/https?:\/\/\S+/g, "").trim()}</span>}
            {link && <button type="button" className="todo__link" onClick={() => void openExternal(link)}><Icon n="link" size={12} />Open link</button>}
          </div>
        )}
      </div>
      <button type="button" className="iconbtn iconbtn--danger todo__del" aria-label="Delete task" title="Delete" onClick={() => void removeTask(t.id)}><Icon n="trash" /></button>
    </div>
  );
}

/** Everyone known from the spaces you share, plus the Workspace directory as you type. */
export function People() {
  const members = useStore((s) => s.members);
  const spaces = useStore((s) => s.spaces);
  const presence = useStore((s) => s.presence);
  const me = useStore((s) => s.auth.user?.name);
  const [q, setQ] = useState("");
  const { hits, busy, failed } = usePeopleSearch(q);
  const known = useMemo(() => {
    const out = new Map<string, { u: User; spaces: number }>();
    for (const [space, list] of Object.entries(members)) {
      const sp = spaces.find((x) => x.name === space);
      for (const m of list) {
        const u = m.member;
        if (!u || u.name === me || u.type === "BOT") continue;
        const row = out.get(u.name) ?? { u, spaces: 0 };
        if (!row.u.displayName && u.displayName) row.u = u;
        if (sp?.spaceType === "SPACE") row.spaces += 1;
        out.set(u.name, row);
      }
    }
    return [...out.values()].sort((a, b) => userLabel(a.u).localeCompare(userLabel(b.u)));
  }, [members, spaces, me]);
  const list = q.trim().length >= 2
    ? hits.filter((u) => u.name !== me).map((u) => ({ u, spaces: known.find((k) => k.u.name === u.name)?.spaces ?? 0 }))
    : known;
  return (
    <>
      <Head title="People" sub={q.trim().length >= 2 ? (busy ? "Searching the directory…" : failed ? "Directory unavailable, showing people from your spaces" : `${list.length} ${list.length === 1 ? "match" : "matches"}`) : `${known.length} people in your spaces`} />
      <div className="feed" role="list" aria-label="People">
        <div className="todo__add people__search">
          <Icon n="search" />
          <input value={q} onChange={(e) => setQ(e.target.value)} placeholder="Search people by name or email" aria-label="Search people" autoFocus />
        </div>
        {list.length === 0 && <div className="feed__empty"><Icon n="users" size={28} /><b>No one here yet</b>People appear as their spaces load, or search the directory above.</div>}
        <div className="people">
          {list.map(({ u, spaces: n }, i) => (
            <button key={u.name} type="button" className="person" style={stagger(i, 20)} onClick={() => void openDm(u)} title={`Message ${userLabel(u)}`}>
              <Avatar user={u} size={40} />
              <div className="person__body">
                <div className="person__name">{userLabel(u)}<Presence state={presence[u.name]?.state ?? "offline"} /></div>
                <div className="person__sub">{u.email ?? ""}{presence[u.name]?.custom_status ? ` · ${presence[u.name]!.custom_status!.emoji ?? ""} ${presence[u.name]!.custom_status!.text}` : n ? ` · ${n} shared ${n === 1 ? "space" : "spaces"}` : ""}</div>
              </div>
              <span className="person__go"><Icon n="dms" size={14} />Message</span>
            </button>
          ))}
        </div>
      </div>
    </>
  );
}
