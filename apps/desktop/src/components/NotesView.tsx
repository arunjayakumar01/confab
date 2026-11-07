import { useEffect, useRef, useState } from "react";
import { addNote, closeFeed, deleteNote, editNote, loadNotes, setNotesQuery, updateNote } from "../actions";
import { stagger } from "../lib/motion";
import { relative, parseTime } from "../lib/time";
import { useStore, type Note } from "../store";
import { Icon } from "./icons";

const COLORS: { id: string; label: string }[] = [
  { id: "none", label: "Default" }, { id: "sun", label: "Sun" }, { id: "mint", label: "Mint" }, { id: "sky", label: "Sky" }, { id: "rose", label: "Rose" }, { id: "lilac", label: "Lilac" },
];

/** Keep-style notes: a quick composer, pinned first, cards that edit in place; the list saves to Drive app data. */
export function NotesView() {
  const { items, loaded, saving, error, query, editing } = useStore((s) => s.notes);
  const q = query.trim().toLowerCase();
  const shown = items.filter((n) => !q || n.title.toLowerCase().includes(q) || n.body.toLowerCase().includes(q));
  const pinned = shown.filter((n) => n.pinned);
  const others = shown.filter((n) => !n.pinned);
  return (
    <>
      <div className="header glass-strong">
        <div className="header__name"><span className="header__name-in">Notes</span></div>
        <div className="header__meta"><span className="header__desc">{!loaded ? "Loading…" : error ? "Notes did not load" : saving ? "Saving…" : `${items.length} ${items.length === 1 ? "note" : "notes"}, saved in your Drive`}</span></div>
        <div className="header__spacer" />
        <div className="notes__search"><Icon n="search" size={14} /><input value={query} onChange={(e) => setNotesQuery(e.target.value)} placeholder="Search notes" aria-label="Search notes" /></div>
        <button type="button" className="iconbtn" aria-label="Close" onClick={closeFeed}><Icon n="x" /></button>
      </div>
      <div className="feed notes">
        <Composer />
        {error && <div className="feed__empty"><Icon n="x" size={28} /><b>Notes did not load</b>{error.includes("403") || error.includes("insufficient") ? "The Drive API needs to be enabled in the project and the sign-in needs the app-data scope." : error}<button type="button" className="btn" onClick={() => { useStore.setState((s) => ({ notes: { ...s.notes, loaded: false } })); void loadNotes(); }}>Try again</button></div>}
        {loaded && !error && items.length === 0 && <div className="feed__empty"><Icon n="edit" size={28} /><b>No notes yet</b>Write the first one above. Notes live in your Google Drive app data and follow you to any Confab.</div>}
        {pinned.length > 0 && <div className="notes__label">Pinned</div>}
        {pinned.length > 0 && <div className="notes__grid">{pinned.map((n, i) => <NoteCard key={n.id} n={n} i={i} editing={editing === n.id} />)}</div>}
        {pinned.length > 0 && others.length > 0 && <div className="notes__label">Others</div>}
        <div className="notes__grid">{others.map((n, i) => <NoteCard key={n.id} n={n} i={i} editing={editing === n.id} />)}</div>
      </div>
    </>
  );
}

function Composer() {
  const [open, setOpen] = useState(false);
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [color, setColor] = useState("none");
  const ref = useRef<HTMLTextAreaElement>(null);
  const done = () => {
    if (title.trim() || body.trim()) addNote(title.trim(), body.trim(), color);
    setTitle(""); setBody(""); setColor("none"); setOpen(false);
  };
  useEffect(() => { if (open) ref.current?.focus(); }, [open]);
  return (
    <div className={`note note--composer note--${color}${open ? " note--open" : ""}`}>
      {open && <input className="note__title" value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Title" aria-label="Title" />}
      <textarea ref={ref} className="note__body" value={body} onChange={(e) => setBody(e.target.value)} onFocus={() => setOpen(true)} placeholder="Take a note…" aria-label="Note" rows={open ? 3 : 1} onKeyDown={(e) => { if (e.key === "Escape") { e.stopPropagation(); done(); } if ((e.metaKey || e.ctrlKey) && e.key === "Enter") done(); }} />
      {open && (
        <div className="note__bar">
          <Palette value={color} onPick={setColor} />
          <span className="header__spacer" />
          <button type="button" className="btn" onClick={done}>{title.trim() || body.trim() ? "Done" : "Close"}</button>
        </div>
      )}
    </div>
  );
}

function Palette({ value, onPick }: { value: string; onPick: (c: string) => void }) {
  return (
    <span className="palette" role="radiogroup" aria-label="Colour">
      {COLORS.map((c) => <button key={c.id} type="button" role="radio" aria-checked={value === c.id} className={`palette__dot palette__dot--${c.id}${value === c.id ? " palette__dot--on" : ""}`} title={c.label} onClick={() => onPick(c.id)} />)}
    </span>
  );
}

function NoteCard({ n, i, editing }: { n: Note; i: number; editing: boolean }) {
  const [title, setTitle] = useState(n.title);
  const [body, setBody] = useState(n.body);
  useEffect(() => { if (!editing) { setTitle(n.title); setBody(n.body); } }, [n.title, n.body, editing]);
  const commit = () => { if (title !== n.title || body !== n.body) updateNote(n.id, { title, body }); editNote(undefined); };
  return (
    <div className={`note note--${n.color}${editing ? " note--editing" : ""}`} style={stagger(i, 16)} onClick={() => { if (!editing) editNote(n.id); }}>
      <button type="button" className={`note__pin${n.pinned ? " note__pin--on" : ""}`} aria-pressed={n.pinned} aria-label={n.pinned ? "Unpin" : "Pin"} onClick={(e) => { e.stopPropagation(); updateNote(n.id, { pinned: !n.pinned }); }}><Icon n="pin" size={14} fill={n.pinned} /></button>
      {editing ? (
        <>
          <input className="note__title" value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Title" aria-label="Title" autoFocus={!n.title && !n.body} />
          <textarea className="note__body" value={body} onChange={(e) => setBody(e.target.value)} placeholder="Note" aria-label="Note" rows={Math.min(14, Math.max(3, body.split("\n").length + 1))} autoFocus={!!(n.title || n.body)} onKeyDown={(e) => { if (e.key === "Escape") { e.stopPropagation(); commit(); } }} />
          <div className="note__bar">
            <Palette value={n.color} onPick={(c) => updateNote(n.id, { color: c })} />
            <button type="button" className="iconbtn iconbtn--danger" aria-label="Delete note" title="Delete" onClick={(e) => { e.stopPropagation(); deleteNote(n.id); }}><Icon n="trash" /></button>
            <span className="header__spacer" />
            <button type="button" className="btn" onClick={(e) => { e.stopPropagation(); commit(); }}>Done</button>
          </div>
        </>
      ) : (
        <>
          {n.title && <div className="note__title-ro">{n.title}</div>}
          <div className="note__text">{n.body || <em>Empty note</em>}</div>
          <div className="note__when">{relative(parseTime(n.updated))}</div>
        </>
      )}
    </div>
  );
}
