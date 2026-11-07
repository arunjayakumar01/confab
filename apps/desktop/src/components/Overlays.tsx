import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { del, openExternal, openViewer, saveAttachment, ui } from "../actions";
import { useLast as keep } from "../lib/motion";
import { bodyOf, spaceLabel as labelOf, threadInfo } from "../store";
import { fileKind, previewKind } from "../lib/files";
import { useExit, useLast } from "../lib/motion";
import { spaceLabel, useStore } from "../store";
import { Icon } from "./icons";
import { Modal } from "./Modal";
import { ViewerBody, fmtSize } from "./Viewers";

/** The one toast at a time, bottom centre; it slides in and fades out. */
export function Toast() {
  const toast = useStore((s) => s.ui.toast);
  const { mounted, exiting } = useExit(!!toast, 140);
  const last = useLast(toast);
  if (!mounted || !last) return null;
  return createPortal(<div className={`toast${exiting ? " toast--out" : ""}`} role="status">{last.text}</div>, document.body);
}

/** Dashed drop target over the whole window while OS files are dragged across it. */
export function DropZone() {
  const over = useStore((s) => s.ui.dragOver);
  const space = useStore((s) => s.spaces.find((x) => x.name === s.ui.activeSpace));
  const members = useStore((s) => (space ? s.members[space.name] : undefined));
  const me = useStore((s) => s.auth.user?.name);
  const { mounted } = useExit(over, 120);
  if (!mounted) return null;
  const label = space ? spaceLabel(space, members, me) : "";
  return createPortal(
    <div className="drop" aria-hidden="true">
      <div className="drop__box">
        <div>
          {space ? `Drop to send to ${space.spaceType === "SPACE" ? "#" : ""}${label}` : "Pick a space first"}
          <span>Images show inline, other files as cards.</span>
        </div>
      </div>
    </div>,
    document.body,
  );
}

/**
 * The in-app file viewer over a dark backdrop: a bar with the file's kind, name and size plus Open, Save and
 * Close, and a body per kind (image, PDF pages, Word, spreadsheet, archive, text, video, audio).
 * Esc, the backdrop or the X close it; an entry opened out of an archive gets a Back button to the archive.
 */
export function Viewer() {
  const v = useStore((s) => s.ui.viewer);
  const shell = useStore((s) => s.shell);
  const { mounted, exiting } = useExit(!!v, 160);
  const last = useLast(v);
  const [size, setSize] = useState<number | undefined>(undefined);
  useEffect(() => setSize(undefined), [last?.src]);
  if (!mounted || !last) return null;
  const close = () => ui({ viewer: undefined });
  const kind = previewKind(last.mime, last.name);
  const info = fileKind(last.mime, last.name);
  const canSave = shell === "desktop" || last.src.startsWith("data:") || last.src.startsWith("blob:");
  const wide = kind === "pdf" || kind === "docx" || kind === "sheet" || kind === "zip" || kind === "text";
  return createPortal(
    <div className={`overlay viewer${wide ? " viewer--doc" : ""}${exiting ? " overlay--out" : ""}`} onMouseDown={(e) => e.target === e.currentTarget && close()} role="dialog" aria-modal="true" aria-label={last.name}>
      <div className="viewer__bar">
        {last.back && <button type="button" className="btn viewer__back" onClick={() => openViewer(last.back!.src, last.back!.name, last.back!.mime)}><Icon n="left" />{last.back.name}</button>}
        <span className="viewer__kind" style={{ ["--k" as string]: info.color }}>{info.label}</span>
        <span className="viewer__name" title={last.name}>{last.name}</span>
        <span className="viewer__meta">{info.name}{size !== undefined ? ` · ${fmtSize(size)}` : ""}</span>
        <span className="viewer__spacer" />
        {last.open && <button type="button" className="btn" onClick={() => void openExternal(last.open!)}><Icon n="external" />Open</button>}
        {canSave && <button type="button" className="btn" onClick={() => void saveAttachment(last.src, last.name)}><Icon n="download" />Save</button>}
        <button type="button" className="iconbtn" aria-label="Close" onClick={close}><Icon n="x" /></button>
      </div>
      <div className="viewer__body" onMouseDown={(e) => e.target === e.currentTarget && close()}>
        <ViewerBody key={last.src} kind={kind} src={last.src} name={last.name} mime={last.mime} onSize={setSize} />
      </div>
    </div>,
    document.body,
  );
}

/** Google Chat's delete: a dialog that says it is for everyone, and counts the thread's replies that go with it. */
export function DeleteDialog({ exiting }: { exiting?: boolean }) {
  const name = keep(useStore((s) => s.ui.deleting));
  const space = name?.split("/").slice(0, 2).join("/") ?? "";
  const sp = useStore((s) => s.spaces.find((x) => x.name === space));
  const members = useStore((s) => s.members[space]);
  const me = useStore((s) => s.auth.user?.name);
  const items = useStore((s) => s.messages[space]?.items);
  const m = items?.find((x) => x.name === name) ?? Object.values(useStore.getState().threads).flatMap((t) => t.items).find((x) => x.name === name);
  const replies = m && !m.threadReply && m.thread?.name && items ? threadInfo(items)[m.thread.name]?.count ?? 0 : 0;
  const where = sp ? (sp.spaceType === "SPACE" ? `#${labelOf(sp, members, me)}` : sp.spaceType === "GROUP_CHAT" ? "this group chat" : "this conversation") : "this space";
  const close = () => ui({ deleting: undefined });
  if (!name) return null;
  return (
    <Modal title="Delete message?" onClose={close} exiting={exiting}>
      <div className="dialog__body">
        <p className="dialog__text">
          This message will be deleted for everyone in {where}.
          {replies > 0 && ` Its ${replies} ${replies === 1 ? "reply" : "replies"} go with it.`}
          {m?.sender?.name && me && m.sender.name !== me && " It was sent by someone else; only a space manager can do this."}
        </p>
        {m && bodyOf(m) && <blockquote className="dialog__quote">{bodyOf(m).length > 240 ? `${bodyOf(m).slice(0, 237)}…` : bodyOf(m)}</blockquote>}
      </div>
      <div className="dialog__actions">
        <button type="button" className="btn" onClick={close} autoFocus>Cancel</button>
        <button type="button" className="btn btn--primary btn--destructive" onClick={() => void del(name, replies > 0)}>Delete</button>
      </div>
    </Modal>
  );
}

const GROUPS: { title: string; keys: [string, string][] }[] = [
  { title: "Navigate", keys: [["⌘K", "Jump to a space or person"], ["⌘[  ⌘]", "Back, forward"], ["⌥↑  ⌥↓", "Previous, next space"], ["⌥⇧↑  ⌥⇧↓", "Previous, next unread"], ["⌘⇧A", "Unreads"], ["⌘⇧S", "Saved"], ["⌘⇧T", "Todos"], ["⌘F", "Search messages"]] },
  { title: "Write", keys: [["↵", "Send"], ["⇧↵", "New line"], ["↑", "Edit your last message"], ["⌘B  ⌘I", "Bold, italic"], ["⌘⇧X  ⌘⇧C", "Strike, code"], ["@  :", "Mention someone, emoji"]] },
  { title: "Window", keys: [["⌘N", "New message"], ["⌘.", "Thread panel"], ["⌘=  ⌘-  ⌘0", "Zoom in, out, reset"], ["⌘/", "This list"], ["esc", "Close whatever is open"]] },
];
export function Shortcuts({ exiting }: { exiting?: boolean }) {
  const close = () => ui({ shortcutsOpen: false });
  return (
    <Modal title="Keyboard shortcuts" onClose={close} exiting={exiting}>
      <div className="dialog__body keys">
        {GROUPS.map((g) => (
          <section key={g.title}>
            <h3>{g.title}</h3>
            {g.keys.map(([k, what]) => (
              <div key={k} className="keys__row"><span className="kbd">{k}</span><span>{what}</span></div>
            ))}
          </section>
        ))}
      </div>
    </Modal>
  );
}
