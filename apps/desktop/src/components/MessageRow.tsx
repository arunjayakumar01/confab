import { useEffect, useRef, useState } from "react";
import type { Attachment, Message, User } from "../bridge-types";
import { edit, markUnread, togglePin, openExternal, openThread, openViewer, taskFromMessage, rememberEmoji, resolveAttachment, retry, saveAttachment, toast, toggleReaction, toggleSaved, ui } from "../actions";
import { fileKind, isInlineImage, needsResolve, previewKind } from "../lib/files";
import { plainText } from "../lib/markdown";
import { useExit, useLast } from "../lib/motion";
import { relative, parseTime, timeShort } from "../lib/time";
import { bodyOf, useStore, type ThreadInfo } from "../store";
import type { Pos } from "./MessageList";
import { Avatar } from "./Avatar";
import { EmojiPicker, type Anchor } from "./EmojiPicker";
import { Icon, type IconName } from "./icons";
import { huddleOf } from "../lib/meet";
import { HuddleCard } from "./HuddleBar";
import { LinkPreviews } from "./LinkPreview";
import { MarkdownView } from "./MarkdownView";
import { Menu, menuAt } from "./Rail";

const NONE: string[] = [];
const FRESH_MS = 4000;
const QUICK_DEFAULT = ["✅", "👀", "🙌"];

export interface Reader { user: User; at: string }

export interface RowProps {
  m: Message;
  collapsed: boolean;
  me?: string;
  members: Record<string, User>;
  thread?: ThreadInfo;
  inThread?: boolean;
  /** People whose read position is this message, newest first; the strip under the message shows them. */
  readers?: Reader[];
  isDm?: boolean;
  /** Set by the list when a jump lands on this row: a one-shot highlight. */
  flash?: boolean;
  /** Place in the sender run; the block card rounds its first and last rows. */
  pos?: Pos;
}

/** One to three emoji and nothing else: shown large, the way Slack does. */
export const isJumbo = (text?: string): boolean =>
  !!text && /^(?:\p{Extended_Pictographic}\uFE0F?(?:\u200d\p{Extended_Pictographic}\uFE0F?)*\s*){1,3}$/u.test(text.trim());

/** `<users/ID>` → `@Name` for the edit box; the reverse happens on save. */
export function toEditable(text: string, members: Record<string, User>): string {
  return text
    .replace(/<chat-user\b[^>]*\bdata-user="(users\/[^"]+)"[^>]*>[^<]*<\/chat-user>/gi, (_, u: string) => `@${members[u]?.displayName ?? u}`)
    .replace(/<(users\/[^>]+)>/g, (_, u: string) => (u === "users/all" ? "@all" : `@${members[u]?.displayName ?? u}`));
}
export function fromEditable(text: string, members: Record<string, User>): string {
  let out = text;
  const list = Object.values(members).filter((u) => u.displayName).sort((a, b) => b.displayName!.length - a.displayName!.length);
  for (const u of list) out = out.split(`@${u.displayName}`).join(`<${u.name}>`);
  return out.replace(/(^|\s)@(all|everyone|channel)\b/g, "$1<users/all>");
}

export const messageLink = (name: string): string => {
  const [, sid, , mid] = name.split("/");
  return `https://mail.google.com/chat/u/0/#chat/space/${sid}/${mid}`;
};

/** What Google Chat leaves in place of a deleted message: who, when, and that it is gone. No menu, no reactions. */
function Tombstone({ m, sender, collapsed, pos }: { m: Message; sender?: User; collapsed: boolean; pos: Pos }) {
  const kind = m.deletionMetadata?.deletionType ?? "";
  const label = kind === "APP_MESSAGE_EXPIRY" ? "Message expired" : kind.startsWith("SPACE_OWNER") || kind === "ADMIN" ? "Message deleted by a space manager" : "Message deleted";
  const t = parseTime(m.createTime);
  return (
    <div className={`msg msg--${pos}${collapsed ? " msg--compact" : ""} msg--deleted`} role="listitem" data-name={m.name}>
      {collapsed ? <span className="msg__gutter">{timeShort(t)}</span> : <Avatar user={sender} size={36} />}
      <div className="msg__body">
        {!collapsed && <div className="msg__head"><span className="msg__name">{sender?.displayName ?? sender?.name ?? "Unknown"}</span><span className="msg__time" title={t.toLocaleString()}>{timeShort(t)}</span></div>}
        <div className="msg__deleted" title={m.deleteTime ? `Deleted ${parseTime(m.deleteTime).toLocaleString()}` : undefined}><Icon n="trash" size={13} />{label}</div>
      </div>
    </div>
  );
}

export function MessageRow({ m, collapsed, me, members, thread, inThread, readers, isDm, flash, pos = "solo" }: RowProps) {
  const editing = useStore((s) => s.ui.editing === m.name);
  const manager = useStore((s) => (s.members[m.name.split("/").slice(0, 2).join("/")] ?? []).some((x) => x.member?.name === s.auth.user?.name && x.role === "ROLE_MANAGER"));
  const mine = useStore((s) => s.myReactions[m.name] ?? NONE);
  const saved = useStore((s) => s.saved.some((x) => x.name === m.name));
  const pinned = useStore((s) => (s.pinned[m.name.split("/").slice(0, 2).join("/")] ?? []).includes(m.name));
  const recent = useStore((s) => s.recentEmoji);
  const [picker, setPicker] = useState<Anchor | null>(null);
  const [ctx, setCtx] = useState<React.CSSProperties | null>(null);
  const [who, setWho] = useState<React.CSSProperties | null>(null);
  const whoMenu = useExit(!!who, 120);
  const whoStyle = useLast(who);
  // A row that mounts within seconds of its message is a live arrival: it rises in and the wash fades.
  const [fresh] = useState(() => Date.now() - parseTime(m.createTime).getTime() < FRESH_MS);
  const pick = useExit(!!picker, 120);
  const anchor = useLast(picker);
  const menu = useExit(!!ctx, 120);
  const menuStyle = useLast(ctx);
  const own = !!me && m.sender?.name === me;
  const canDelete = own || manager;
  const authUser = useStore((s) => s.auth.user);
  const sender: User | undefined = m.sender && (m.sender.displayName ? m.sender : members[m.sender.name] ?? (m.sender.name === me ? authUser : m.sender));
  const t = parseTime(m.createTime);
  if (m.deleteTime) return <Tombstone m={m} sender={sender} collapsed={collapsed} pos={pos} />;
  const edited = !!m.lastUpdateTime && m.lastUpdateTime !== m.createTime;
  const pending = m._pending;
  const sums = m.emojiReactionSummaries ?? [];
  const quick = [...recent, ...QUICK_DEFAULT.filter((u) => !recent.includes(u))].slice(0, 3);
  const react = (u: string) => { rememberEmoji(u); void toggleReaction(m.name, u); };
  const openPicker = (e: React.MouseEvent) => {
    const r = (e.currentTarget as HTMLElement).getBoundingClientRect();
    setPicker({ x: r.left + r.width / 2, y: r.bottom });
  };
  const copyLink = () => {
    void navigator.clipboard?.writeText(messageLink(m.name));
    toast("Link copied");
  };
  const copyText = () => {
    void navigator.clipboard?.writeText(plainText(bodyOf(m), (u) => members[u]?.displayName ?? u));
    toast("Message copied");
  };
  const onContextMenu = (e: React.MouseEvent) => {
    if (editing || pending) return;
    // Links and inputs keep the native menu; the row itself gets ours.
    if ((e.target as HTMLElement).closest("a, textarea, input")) return;
    e.preventDefault();
    setCtx(menuAt(e));
  };
  const cls = ["msg", `msg--${pos}`, collapsed && "msg--compact", isJumbo(bodyOf(m)) && !m.attachment?.length && "msg--jumbo", pending && "msg--pending", editing && "msg--editing", (picker || ctx || who) && "msg--menu", fresh && "msg--new", flash && "msg--flash"].filter(Boolean).join(" ");
  const items: ({ sep: true } | { icon: IconName; label: string; danger?: boolean; hint?: string; run: () => void })[] = [
    { icon: "reply", label: "Reply in thread", run: () => m.thread?.name && openThread(m.thread.name) },
    { icon: "bookmark", label: saved ? "Remove from saved" : "Save for later", run: () => toggleSaved(m) },
    { icon: "pin", label: pinned ? "Unpin from space" : "Pin to space", run: () => togglePin(m.name) },
    { icon: "eye", label: "Mark unread from here", run: () => markUnread(m.name) },
    { icon: "check", label: "Add to Tasks", run: () => void taskFromMessage(m, members) },
    { sep: true },
    { icon: "copy", label: "Copy text", run: copyText },
    { icon: "link", label: "Copy link", run: copyLink },
    ...(own || canDelete ? [{ sep: true } as const] : []),
    ...(own ? [{ icon: "edit" as IconName, label: "Edit message", run: () => ui({ editing: m.name }) }] : []),
    ...(canDelete ? [{ icon: "trash" as IconName, label: "Delete message", danger: true, run: () => ui({ deleting: m.name }) }] : []),
  ];
  return (
    <div className={cls} role="listitem" data-name={m.name} onContextMenu={onContextMenu}>
      {collapsed ? <span className="msg__gutter">{timeShort(t)}</span> : <Avatar user={sender} size={36} />}
      <div className="msg__body">
        {!collapsed && (
          <div className="msg__head">
            <span className="msg__name">{sender?.displayName ?? sender?.name ?? "Unknown"}</span>
            <span className="msg__time" title={t.toLocaleString()}>{timeShort(t)}</span>
            {saved && <Icon n="bookmark" size={12} fill />}
            {pinned && <span className="msg__pinned"><Icon n="pin" size={11} />Pinned</span>}
          </div>
        )}
        {editing ? (
          <EditBox m={m} members={members} />
        ) : (
          <>
            {bodyOf(m) && !huddleOf(m)?.huddle && <MarkdownView text={bodyOf(m)} members={members} me={me} fold />}
            {huddleOf(m) && <HuddleCard m={m} />}
            {!editing && !huddleOf(m) && <LinkPreviews message={m} />}
            {edited && <span className="msg__edited">(edited)</span>}
            {m.attachment && m.attachment.length > 0 && (
              <div className="att">
                {m.attachment.map((a, i) => <AttachmentView key={a.name ?? i} a={a} message={m.name} />)}
              </div>
            )}
            {sums.length > 0 && (
              <div className="reactions">
                {sums.map((r) => {
                  const u = r.emoji?.unicode ?? "";
                  const isMine = mine.includes(u);
                  const n = r.reactionCount ?? 0;
                  return (
                    <button key={u} type="button" className={`reaction${isMine ? " reaction--mine" : ""}`} aria-pressed={isMine} aria-label={`${u} ${n}`} onClick={() => react(u)}>
                      <span>{u}</span>
                      <span key={n} className="reaction__n">{n}</span>
                    </button>
                  );
                })}
                <button type="button" className="reaction reaction--add" aria-label="Add reaction" onClick={openPicker}>
                  <Icon n="smile" />
                  <Icon n="plus" />
                </button>
              </div>
            )}
            {!inThread && thread && thread.count > 0 && m.thread?.name && (
              <button type="button" className="replies" onClick={() => openThread(m.thread!.name!)}>
                <span className="replies__avs">
                  {thread.repliers.slice(0, 3).map((u) => <Avatar key={u.name} user={u} size={22} />)}
                </span>
                <span className="replies__n">{thread.count} {thread.count === 1 ? "reply" : "replies"}</span>
                <span className="replies__last">Last reply {relative(parseTime(thread.last))}</span>
                <span className="replies__arrow">View thread</span>
              </button>
            )}
            {pending === "failed" && (
              <div className="msg__fail" role="alert">
                Couldn't send{m._error ? `: ${m._error}` : ""}.
                <button type="button" onClick={() => void retry(m)}>Retry</button>
              </div>
            )}
            {readers && readers.length > 0 && (
              <button type="button" className="seen" title={`Seen by ${readers.map((r) => r.user.displayName ?? r.user.name).join(", ")}`} aria-label={`Seen by ${readers.length}`} onClick={(e) => { const r = e.currentTarget.getBoundingClientRect(); setWho(menuAt({ clientX: r.right - 232, clientY: r.bottom + 4 }, 232, 200)); }}>
                {isDm && <span>Seen</span>}
                <span className="seen__avs">{readers.slice(0, 5).map((r) => <Avatar key={r.user.name} user={r.user} size={16} />)}</span>
                {readers.length > 5 && <span>+{readers.length - 5}</span>}
              </button>
            )}
          </>
        )}
      </div>
      {!editing && !pending && (
        <div className="actions" role="toolbar" aria-label="Message actions">
          {(
            <>
              {quick.map((u) => (
                <button key={u} type="button" className={`iconbtn actions__quick${mine.includes(u) ? " actions__quick--on" : ""}`} aria-label={`React ${u}`} title={`React ${u}`} onClick={() => react(u)}>{u}</button>
              ))}
              <button type="button" className="iconbtn" aria-label="React" title="Find another reaction" onClick={openPicker}><Icon n="smile" /></button>
              {!inThread && m.thread?.name && (
                <button type="button" className="iconbtn" aria-label="Reply in thread" title="Reply in thread" onClick={() => openThread(m.thread!.name!)}><Icon n="reply" /></button>
              )}
              <button type="button" className={`iconbtn${saved ? " iconbtn--on" : ""}`} aria-label={saved ? "Remove from saved" : "Save for later"} title={saved ? "Remove from saved" : "Save for later"} aria-pressed={saved} onClick={() => toggleSaved(m)}><Icon n="bookmark" fill={saved} /></button>
              {own && <button type="button" className="iconbtn" aria-label="Edit" title="Edit" onClick={() => ui({ editing: m.name })}><Icon n="edit" /></button>}
              {canDelete && <button type="button" className="iconbtn iconbtn--danger" aria-label="Delete" title="Delete" onClick={() => ui({ deleting: m.name })}><Icon n="trash" /></button>}
              <button type="button" className="iconbtn" aria-label="More actions" title="More" onClick={(e) => { const r = e.currentTarget.getBoundingClientRect(); setCtx(menuAt({ clientX: r.right - 232, clientY: r.bottom + 4 })); }}><Icon n="more" /></button>
            </>
          )}
        </div>
      )}
      {pick.mounted && anchor && (
        <EmojiPicker
          at={anchor}
          exiting={pick.exiting}
          onClose={() => setPicker(null)}
          onPick={(u) => {
            setPicker(null);
            react(u);
          }}
        />
      )}
      {whoMenu.mounted && whoStyle && readers && (
        <Menu className="menu--ctx menu--who" label="Seen by" style={whoStyle} exiting={whoMenu.exiting} onClose={() => setWho(null)}>
          <div className="menu__title">Seen by</div>
          {readers.map((r) => (
            <div key={r.user.name} className="menu__item menu__item--static">
              <Avatar user={r.user} size={22} />
              <span>{r.user.displayName ?? r.user.name}</span>
              <span className="menu__when">{relative(parseTime(r.at))}</span>
            </div>
          ))}
        </Menu>
      )}
      {menu.mounted && menuStyle && (
        <Menu className="menu--ctx" label="Message actions" style={menuStyle} exiting={menu.exiting} onClose={() => setCtx(null)}>
          <div className="menu__quick" role="group" aria-label="React">
            {[...quick, ...["🎉", "❤️", "😂", "🔥"].filter((u) => !quick.includes(u))].slice(0, 6).map((u) => (
              <button key={u} type="button" className={mine.includes(u) ? "on" : undefined} aria-label={`React ${u}`} onClick={() => { setCtx(null); react(u); }}>{u}</button>
            ))}
          </div>
          <div className="menu__sep" />
          {items.map((it, i) =>
            "sep" in it ? <div key={i} className="menu__sep" /> : (
              <button key={it.label} type="button" role="menuitem" className={`menu__item${it.danger ? " menu__item--danger" : ""}`} onClick={() => { setCtx(null); it.run(); }}>
                <Icon n={it.icon} />{it.label}
              </button>
            ),
          )}
        </Menu>
      )}
    </div>
  );
}

/** An image inline, anything else as a card with a kind badge. Drive stubs fetch their details on first sight. */
function AttachmentView({ a, message }: { a: Attachment; message: string }) {
  const shell = useStore((s) => s.shell);
  const loading = needsResolve(a);
  useEffect(() => {
    if (loading) void resolveAttachment(message, a);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [a.name, loading]);
  const url = a.downloadUri ?? a.thumbnailUri;
  const http = !!url && /^https?:/i.test(url);
  // Uploaded files come through the shell proxy (or data: URLs in the mock) and preview in the app; Drive links need
  // Google's own session, so they open in the browser.
  const viewable = !!url && !!previewKind(a.contentType, a.contentName) && a.source !== "DRIVE_FILE";
  const open = () => {
    if (!url) { toast("Nothing to open for this file"); return; }
    if (viewable) openViewer(url, a.contentName ?? "file", a.contentType, http ? url : undefined);
    else if (http) void openExternal(url);
    else if (shell === "desktop" || url.startsWith("data:") || url.startsWith("blob:")) void saveAttachment(url, a.contentName ?? "file");
    else void openExternal(url);
  };
  if (isInlineImage(a)) {
    const src = a.thumbnailUri ?? a.downloadUri!;
    return (
      <button type="button" className="att__imgbtn" onClick={() => openViewer(src, a.contentName ?? "image", a.contentType, http ? url : undefined)} title={a.contentName ?? "image"}>
        <img className="att__img" src={src} alt={a.contentName ?? "image"} loading="lazy" />
      </button>
    );
  }
  const kind = fileKind(a.contentType, a.contentName);
  return (
    <button type="button" className={`att__file${loading ? " att__file--loading" : ""}`} onClick={open} disabled={loading} title={a.contentName}>
      <span className="att__kind" style={{ ["--k" as string]: loading ? "var(--fg-3)" : kind.color }}>{loading ? "…" : kind.label}</span>
      <span className="att__meta">
        <span className="att__name">{loading ? "" : a.contentName ?? "File"}</span>
        <span className="att__sub">
          {loading ? "Fetching details" : (
            <>
              {kind.name}{a.source === "DRIVE_FILE" ? " · Drive" : ""}
              {url && <Icon n={viewable ? "eye" : http ? "external" : "download"} />}
            </>
          )}
        </span>
      </span>
    </button>
  );
}

function EditBox({ m, members }: { m: Message; members: Record<string, User> }) {
  const [text, setText] = useState(() => toEditable(bodyOf(m), members));
  const ref = useRef<HTMLTextAreaElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.focus();
    el.setSelectionRange(el.value.length, el.value.length);
    el.style.height = "auto";
    el.style.height = `${el.scrollHeight}px`;
  }, []);
  const save = () => {
    const v = fromEditable(text.trim(), members);
    if (!v) return;
    if (v === bodyOf(m)) ui({ editing: undefined });
    else void edit(m.name, v);
  };
  return (
    <div className="edit">
      <textarea
        ref={ref}
        value={text}
        aria-label="Edit message"
        onChange={(e) => {
          setText(e.target.value);
          e.target.style.height = "auto";
          e.target.style.height = `${e.target.scrollHeight}px`;
        }}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            e.stopPropagation();
            ui({ editing: undefined });
          } else if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            save();
          }
        }}
      />
      <div className="edit__hint"><b>Enter</b> to save, <b>Esc</b> to cancel</div>
    </div>
  );
}
