import { useMemo, useState, type FormEvent } from "react";
import type { Attachment, Message, Space, User } from "../bridge-types";
import { addMembers, jumpTo, loadOlder, openFeed, openViewer, setPanel, startMeet, togglePin, ui } from "../actions";
import { fileKind, isInlineImage, previewKind } from "../lib/files";
import { stagger } from "../lib/motion";
import { dateShort, parseTime } from "../lib/time";
import { nameMap, spaceLabel, useStore } from "../store";
import { Avatar } from "./Avatar";
import { Icon } from "./icons";
import { MessageRow } from "./MessageRow";
import { Modal } from "./Modal";
import { PeoplePicker } from "./PeoplePicker";
import { positions } from "./MessageList";

function PanelFrame({ title, sub, exiting, label, children }: { title: string; sub?: string; exiting?: boolean; label: string; children: React.ReactNode }) {
  return (
    <aside className={`panel glass${exiting ? " panel--out" : ""}`} aria-label={label}>
      <div className="panel__head">
        <span className="panel__title">{title}</span>
        {sub && <span className="panel__sub">{sub}</span>}
        <button type="button" className="iconbtn" aria-label={`Close ${title.toLowerCase()}`} onClick={() => setPanel("none")}><Icon n="x" /></button>
      </div>
      {children}
    </aside>
  );
}

/** Everything shared in the space, from the messages loaded so far: images as a grid, the rest as cards. */
export function FilesPanel({ space, exiting }: { space: Space; exiting?: boolean }) {
  const page = useStore((s) => s.messages[space.name]);
  const members = useStore((s) => s.members[space.name]);
  const me = useStore((s) => s.auth.user?.name);
  const shell = useStore((s) => s.shell);
  const files = useMemo(() => {
    const out: { a: Attachment; m: Message }[] = [];
    for (const m of [...(page?.items ?? [])].reverse()) for (const a of m.attachment ?? []) out.push({ a, m });
    return out;
  }, [page?.items]);
  const images = files.filter((f) => isInlineImage(f.a));
  const docs = files.filter((f) => !isInlineImage(f.a));
  const open = (a: Attachment) => {
    const url = a.downloadUri ?? a.thumbnailUri;
    if (!url) return;
    const http = /^https?:/i.test(url);
    if (previewKind(a.contentType, a.contentName) && a.source !== "DRIVE_FILE") openViewer(url, a.contentName ?? "file", a.contentType, http ? url : undefined);
    else if (http) void import("../actions").then((x) => x.openExternal(url));
    else if (shell === "desktop") void import("../actions").then((x) => x.saveAttachment(url, a.contentName ?? "file"));
  };
  return (
    <PanelFrame title="Files" sub={`${files.length} in ${space.spaceType === "SPACE" ? `#${spaceLabel(space, members, me)}` : spaceLabel(space, members, me)}`} exiting={exiting} label="Files">
      <div className="panel__scroll files">
        {files.length === 0 && <div className="feed__empty"><Icon n="clip" size={28} /><b>Nothing shared yet</b>Files and images sent here collect in this panel.</div>}
        {images.length > 0 && (
          <>
            <div className="files__head">Images</div>
            <div className="files__grid">
              {images.map(({ a, m }, i) => (
                <button key={`${m.name}:${a.name ?? i}`} type="button" className="files__img" style={stagger(i)} onClick={() => open(a)} title={a.contentName}>
                  <img src={a.thumbnailUri ?? a.downloadUri} alt={a.contentName ?? "image"} loading="lazy" />
                </button>
              ))}
            </div>
          </>
        )}
        {docs.length > 0 && <div className="files__head">Files</div>}
        {docs.map(({ a, m }, i) => {
          const k = fileKind(a.contentType, a.contentName);
          return (
            <div key={`${m.name}:${a.name ?? i}`} className="files__row" style={stagger(i)}>
              <button type="button" className="files__open" onClick={() => open(a)} title={a.contentName}>
                <span className="att__kind" style={{ ["--k" as string]: k.color }}>{k.label}</span>
                <span className="files__meta">
                  <span className="att__name">{a.contentName ?? "File"}</span>
                  <span className="att__sub">{m.sender?.displayName ?? "Someone"} · {dateShort(parseTime(m.createTime))}</span>
                </span>
              </button>
              <button type="button" className="iconbtn" aria-label="Go to message" title="Go to message" onClick={() => jumpTo(m.name)}><Icon n="reply" /></button>
            </div>
          );
        })}
        {page?.nextPageToken && (
          <button type="button" className="btn files__more" disabled={page.loading} onClick={() => void loadOlder(space.name)}>{page.loading ? "Loading…" : "Look further back"}</button>
        )}
      </div>
    </PanelFrame>
  );
}

/** Messages pinned in this space. Local to this device: Google Chat has no pin API. */
export function PinnedPanel({ space, exiting }: { space: Space; exiting?: boolean }) {
  const names = useStore((s) => s.pinned[space.name] ?? EMPTY);
  const items = useStore((s) => s.messages[space.name]?.items);
  const members = useStore((s) => s.members[space.name]);
  const me = useStore((s) => s.auth.user?.name);
  const map = useMemo(() => nameMap(members), [members]);
  const msgs = names.map((n) => items?.find((m) => m.name === n)).filter((m): m is Message => !!m);
  const pos = positions(msgs.map((m) => ({ ...m, attachment: [] })));
  return (
    <PanelFrame title="Pinned" sub={`${names.length} on this device`} exiting={exiting} label="Pinned messages">
      <div className="panel__scroll">
        {names.length === 0 && <div className="feed__empty"><Icon n="star" size={28} /><b>Nothing pinned</b>Pin from a message's menu. Pins stay on this device; Google has no pin for user apps.</div>}
        {msgs.map((m, i) => (
          <div key={m.name} className="pinned">
            <MessageRow m={m} collapsed={false} pos={pos[i]} me={me} members={map} inThread />
            <div className="pinned__bar">
              <button type="button" className="btn" onClick={() => jumpTo(m.name)}><Icon n="reply" />Go to message</button>
              <button type="button" className="btn" onClick={() => togglePin(m.name)}>Unpin</button>
            </div>
          </div>
        ))}
        {names.length > msgs.length && <div className="list__top">{names.length - msgs.length} pinned {names.length - msgs.length === 1 ? "message is" : "messages are"} older than what is loaded.</div>}
      </div>
    </PanelFrame>
  );
}
const EMPTY: string[] = [];

/** Google Chat's "Add members": pick people, they join the space. */
export function AddMembersDialog({ space, exiting }: { space: Space; exiting?: boolean }) {
  const members = useStore((s) => s.members[space.name]);
  const me = useStore((s) => s.auth.user?.name);
  const [people, setPeople] = useState<User[]>([]);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const close = () => ui({ addMembersOpen: false });
  const go = (e: FormEvent) => {
    e.preventDefault();
    if (!people.length || busy) return;
    setBusy(true);
    setErr("");
    addMembers(space.name, people).then(close).catch((x: unknown) => { setErr((x as Error)?.message ?? String(x)); setBusy(false); });
  };
  const label = spaceLabel(space, members, me);
  return (
    <Modal title={`Add members to ${space.spaceType === "SPACE" ? `#${label}` : label}`} onClose={close} exiting={exiting}>
      <form onSubmit={go}>
        <div className="dialog__body">
          <div className="field">
            <span>People</span>
            <PeoplePicker value={people.filter((u) => !(members ?? []).some((m) => m.member?.name === u.name))} onChange={setPeople} autoFocus />
          </div>
          <div className="dialog__note">They get the space's history and can read everything in it.</div>
          {err && <div className="dialog__note dialog__note--err" role="alert">{err}</div>}
        </div>
        <div className="dialog__actions">
          <button type="button" className="btn" onClick={close} disabled={busy}>Cancel</button>
          <button type="submit" className="btn btn--primary" disabled={busy || !people.length}>{busy ? "Adding…" : people.length > 1 ? `Add ${people.length} people` : "Add"}</button>
        </div>
      </form>
    </Modal>
  );
}

/** The empty-space card Google Chat shows: what to do first. */
export function Welcome({ space }: { space: Space }) {
  const members = useStore((s) => s.members[space.name]);
  const me = useStore((s) => s.auth.user?.name);
  const first = useStore((s) => s.auth.user?.displayName?.split(" ")[0]);
  const label = spaceLabel(space, members, me);
  const isDm = space.spaceType === "DIRECT_MESSAGE";
  return (
    <div className="list list__empty">
      <div className="welcome">
        <div className="welcome__mark">{isDm ? <Avatar user={(members ?? []).map((m) => m.member).find((u) => u && u.name !== me)} size={48} /> : <span>#</span>}</div>
        <b>{isDm ? `This is the start of your conversation with ${label}` : `${first ? `${first}, welcome` : "Welcome"} to ${space.spaceType === "SPACE" ? `#${label}` : label}`}</b>
        <span>{isDm ? "Say hello, share a file, or start a call." : "Let's get started:"}</span>
        <div className="welcome__chips">
          {!isDm && <button type="button" className="welcome__chip" onClick={() => ui({ addMembersOpen: true })}><Icon n="users" />Add members</button>}
          <button type="button" className="welcome__chip" onClick={() => ui({ pickFile: Date.now() })}><Icon n="clip" />Share a file</button>
          <button type="button" className="welcome__chip" onClick={() => openFeed("todos")}><Icon n="check" />Assign tasks</button>
          <button type="button" className="welcome__chip" onClick={() => void startMeet(space.name)}><Icon n="video" />Start a video call</button>
        </div>
      </div>
    </div>
  );
}
