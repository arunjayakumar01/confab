import { useEffect, useMemo, useRef, useState } from "react";
import type { User } from "../bridge-types";
import { openExternal, rememberEmoji, saveDraft, send, startMeet, toast, typing, ui, uploadAndSend } from "../actions";
import { Menu } from "./Rail";
import { searchEmoji } from "../lib/emoji";
import { fuzzyScore } from "../lib/fuzzy";
import { useExit, useLast } from "../lib/motion";
import { draftKey, nameMap, useStore } from "../store";
import { Avatar } from "./Avatar";
import { EmojiPicker, type Anchor } from "./EmojiPicker";
import { Icon, type IconName } from "./icons";
import { fromEditable } from "./MessageRow";

const TYPING_EVERY_MS = 3000;

interface Props { space: string; thread?: string; placeholder: string; autoFocus?: boolean }

export function Composer({ space, thread, placeholder, autoFocus }: Props) {
  const key = draftKey(space, thread);
  const draft = useStore((s) => s.drafts[key] ?? "");
  const members = useStore((s) => s.members[space]);
  const me = useStore((s) => s.auth.user?.name);
  const items = useStore((s) => (thread ? s.threads[thread]?.items : s.messages[space]?.items));
  const names = useMemo(() => nameMap(members), [members]);
  const [text, setText] = useState(draft);
  const [picker, setPicker] = useState<Anchor | null>(null);
  const [fly, setFly] = useState(false);
  // One popup for both triggers: "@" completes people, ":" completes emoji shortcodes.
  const [mention, setMention] = useState<{ start: number; q: string; idx: number; kind: "user" | "emoji" } | null>(null);
  const ref = useRef<HTMLTextAreaElement>(null);
  const fileRef = useRef<HTMLInputElement>(null);
  const lastTyping = useRef(0);
  const textRef = useRef(text);
  textRef.current = text;
  const pick = useExit(!!picker, 120);
  const anchor = useLast(picker);
  const [plus, setPlus] = useState(false);
  const plusMenu = useExit(plus, 120);
  const pickTick = useStore((s) => s.ui.pickFile);
  // The welcome card's "Share a file" opens this composer's picker (the main one, not a thread's).
  useEffect(() => { if (pickTick && !thread) fileRef.current?.click(); }, [pickTick, thread]);

  // Restore the draft when switching space/thread; flush the previous one.
  useEffect(() => {
    setText(useStore.getState().drafts[key] ?? "");
    setMention(null);
    lastTyping.current = 0;
    if (autoFocus) ref.current?.focus();
    return () => {
      if (lastTyping.current) typing(space, thread, false);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, window.innerHeight * 0.4)}px`;
  }, [text]);

  const update = (v: string) => {
    setText(v);
    saveDraft(space, thread, v);
    const now = Date.now();
    if (v.trim()) {
      if (now - lastTyping.current > TYPING_EVERY_MS) {
        lastTyping.current = now;
        typing(space, thread, true);
      }
    } else if (lastTyping.current) {
      lastTyping.current = 0;
      typing(space, thread, false);
    }
    // Trigger detection: "@" or ":" at a word start before the caret, with the query running to the caret.
    const el = ref.current;
    const caret = el?.selectionStart ?? v.length;
    const before = v.slice(0, caret);
    const at = before.lastIndexOf("@");
    if (at >= 0 && (at === 0 || /\s/.test(before[at - 1]!)) && !/\s{2,}/.test(before.slice(at))) {
      const q = before.slice(at + 1);
      if (q.length <= 30 && !q.includes("\n")) {
        setMention((m) => ({ start: at, q, idx: m?.q === q ? m.idx : 0, kind: "user" }));
        return;
      }
    }
    const colon = before.lastIndexOf(":");
    if (colon >= 0 && (colon === 0 || /\s/.test(before[colon - 1]!))) {
      const q = before.slice(colon + 1);
      if (/^[a-z0-9_+-]{2,20}$/i.test(q)) {
        setMention((m) => ({ start: colon, q, idx: m?.q === q ? m.idx : 0, kind: "emoji" }));
        return;
      }
    }
    setMention(null);
  };

  const emojiHits = useMemo(() => (mention?.kind === "emoji" ? searchEmoji(mention.q).slice(0, 8) : []), [mention]);
  const candidates = useMemo(() => {
    if (!mention || mention.kind !== "user") return [];
    const all = Object.values(names);
    const scored = all
      .map((u) => ({ u, s: fuzzyScore(mention.q, u.displayName ?? u.email ?? u.name) }))
      .filter((x) => x.s >= 0)
      .sort((a, b) => b.s - a.s)
      .slice(0, 6)
      .map((x) => x.u);
    if (!thread && "all".startsWith(mention.q.toLowerCase())) scored.push({ name: "users/all", displayName: "all" });
    return scored;
  }, [mention, names, me, thread]);

  const insertMention = (u: User) => insertAtTrigger(u.name === "users/all" ? "@all " : `@${u.displayName ?? u.name} `);
  const insertEmoji = (u: string) => { rememberEmoji(u); insertAtTrigger(`${u} `); };
  /** Replaces the trigger and its query with `label` and parks the caret after it. */
  const insertAtTrigger = (label: string) => {
    if (!mention) return;
    const el = ref.current;
    const caret = el?.selectionStart ?? text.length;
    const next = text.slice(0, mention.start) + label + text.slice(caret);
    setMention(null);
    setText(next);
    saveDraft(space, thread, next);
    requestAnimationFrame(() => {
      const pos = mention.start + label.length;
      el?.setSelectionRange(pos, pos);
      el?.focus();
    });
  };

  const submit = () => {
    const raw = text.trim();
    if (!raw) return;
    const out = fromEditable(raw, names);
    setText("");
    setMention(null);
    lastTyping.current = 0;
    saveDraft(space, thread, "");
    setFly(true);
    window.setTimeout(() => setFly(false), 480);
    void send(space, out, thread);
  };

  const wrap = (open: string, close = open, block = false) => {
    const el = ref.current;
    if (!el) return;
    const s = el.selectionStart;
    const e = el.selectionEnd;
    const sel = text.slice(s, e) || (block ? "" : "text");
    const ins = block ? `\n${open}\n${sel}\n${close}\n` : `${open}${sel}${close}`;
    const next = text.slice(0, s) + ins + text.slice(e);
    update(next);
    requestAnimationFrame(() => {
      el.focus();
      const a = s + (block ? open.length + 2 : open.length);
      el.setSelectionRange(a, a + sel.length);
    });
  };
  const bullet = () => {
    const el = ref.current;
    if (!el) return;
    const s = el.selectionStart;
    const lineStart = text.lastIndexOf("\n", s - 1) + 1;
    const next = text.slice(0, lineStart) + "- " + text.slice(lineStart);
    update(next);
    requestAnimationFrame(() => el.setSelectionRange(s + 2, s + 2));
  };
  const link = () => {
    const el = ref.current;
    if (!el) return;
    const sel = text.slice(el.selectionStart, el.selectionEnd);
    const url = window.prompt("Link URL", /^https?:/.test(sel) ? sel : "https://");
    if (!url) return;
    const label = sel && !/^https?:/.test(sel) ? sel : "";
    const ins = label ? `<${url}|${label}>` : url;
    update(text.slice(0, el.selectionStart) + ins + text.slice(el.selectionEnd));
  };
  const insertAt = (s: string) => {
    const el = ref.current;
    const pos = el?.selectionStart ?? text.length;
    const next = text.slice(0, pos) + s + text.slice(pos);
    update(next);
    requestAnimationFrame(() => {
      el?.focus();
      el?.setSelectionRange(pos + s.length, pos + s.length);
    });
  };
  const attach = (f: File | undefined) => {
    if (!f) return;
    toast(`Uploading ${f.name}…`);
    void uploadAndSend(space, f, thread).catch((err) => toast(`Upload failed: ${String(err)}`));
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    const n = mention?.kind === "emoji" ? emojiHits.length : candidates.length;
    if (mention && n) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        setMention({ ...mention, idx: (mention.idx + (e.key === "ArrowDown" ? 1 : n - 1)) % n });
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        if (mention.kind === "emoji") insertEmoji((emojiHits[mention.idx] ?? emojiHits[0]!).u);
        else insertMention(candidates[mention.idx] ?? candidates[0]!);
        return;
      }
      if (e.key === "Escape") {
        e.stopPropagation();
        setMention(null);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit();
      return;
    }
    if (e.key === "ArrowUp" && !text && items) {
      const mineLast = [...items].reverse().find((m) => m.sender?.name === me && !m._pending && (thread ? true : !m.threadReply));
      if (mineLast) {
        e.preventDefault();
        ui({ editing: mineLast.name });
      }
      return;
    }
    const mod = e.metaKey || e.ctrlKey;
    if (mod && e.key === "b") { e.preventDefault(); wrap("*"); }
    else if (mod && e.key === "i") { e.preventDefault(); wrap("_"); }
    else if (mod && e.shiftKey && e.key.toLowerCase() === "x") { e.preventDefault(); wrap("~"); }
    else if (mod && e.shiftKey && e.key.toLowerCase() === "c") { e.preventDefault(); wrap("`"); }
  };

  const fmt: { n: IconName; label: string; f: () => void }[] = [
    { n: "bold", label: "Bold (⌘B)", f: () => wrap("*") },
    { n: "italic", label: "Italic (⌘I)", f: () => wrap("_") },
    { n: "strike", label: "Strikethrough (⌘⇧X)", f: () => wrap("~") },
    { n: "code", label: "Code (⌘⇧C)", f: () => wrap("`") },
    { n: "link", label: "Link", f: link },
    { n: "list", label: "Bulleted list", f: bullet },
  ];
  const ready = !!text.trim();
  return (
    <div className={`composer${thread ? " composer--thread" : ""}`}>
      {mention?.kind === "emoji" && emojiHits.length > 0 && (
        <div className="mentions mentions--emoji" role="listbox" aria-label="Emoji">
          {emojiHits.map((em, i) => (
            <button key={em.u} type="button" role="option" aria-selected={i === mention.idx} className={`mentions__item${i === mention.idx ? " mentions__item--on" : ""}`} onMouseDown={(e) => e.preventDefault()} onClick={() => insertEmoji(em.u)}>
              <span className="mentions__emoji">{em.u}</span>
              <span>:{em.n.split(" ")[0]}:</span>
              <span className="mentions__email">{em.n.split(" ").slice(1).join(" ")}</span>
            </button>
          ))}
        </div>
      )}
      {mention?.kind === "user" && candidates.length > 0 && (
        <div className="mentions" role="listbox" aria-label="Mention someone">
          {candidates.map((u, i) => (
            <button key={u.name} type="button" role="option" aria-selected={i === mention.idx} className={`mentions__item${i === mention.idx ? " mentions__item--on" : ""}`} onMouseDown={(e) => e.preventDefault()} onClick={() => insertMention(u)}>
              {u.name === "users/all" ? <span className="row__icon"><Icon n="users" /></span> : <Avatar user={u} size={22} />}
              <span>{u.displayName ?? u.name}</span>
              <span className="mentions__email">{u.name === "users/all" ? "Notify everyone" : u.email ?? ""}</span>
            </button>
          ))}
        </div>
      )}
      <div className="composer__box">
        <div className="composer__fmt" role="toolbar" aria-label="Formatting">
          {fmt.map((b, i) => (
            <span key={b.n} style={{ display: "contents" }}>
              {i === 4 && <span className="sep" />}
              <button type="button" className="iconbtn" aria-label={b.label} title={b.label} onMouseDown={(e) => e.preventDefault()} onClick={b.f}>
                <Icon n={b.n} />
              </button>
            </span>
          ))}
        </div>
        <textarea
          ref={ref}
          rows={1}
          value={text}
          placeholder={placeholder}
          aria-label={placeholder}
          onChange={(e) => update(e.target.value)}
          onKeyDown={onKeyDown}
          onPaste={(e) => {
            const f = Array.from(e.clipboardData.files)[0];
            if (f) {
              e.preventDefault();
              attach(f);
              return;
            }
            // A URL pasted over selected words links them.
            const el = e.currentTarget;
            const sel = text.slice(el.selectionStart, el.selectionEnd);
            const pasted = e.clipboardData.getData("text").trim();
            if (sel && !/^https?:/.test(sel) && /^https?:\/\/\S+$/.test(pasted)) {
              e.preventDefault();
              update(text.slice(0, el.selectionStart) + `<${pasted}|${sel}>` + text.slice(el.selectionEnd));
            }
          }}
        />
        <div className="composer__bar">
          <button type="button" className={`iconbtn composer__plus${plus ? " iconbtn--on" : ""}`} aria-label="More ways to add" title="Add" aria-haspopup="menu" aria-expanded={plus} onClick={() => setPlus((p) => !p)}>
            <Icon n="plus" />
          </button>
          {plusMenu.mounted && (
            <Menu className={`menu--plus${thread ? " menu--plus-thread" : ""}`} label="Add to the message" exiting={plusMenu.exiting} onClose={() => setPlus(false)}>
              <button type="button" role="menuitem" className="menu__item" onClick={() => { setPlus(false); fileRef.current?.click(); }}><Icon n="clip" />Upload a file</button>
              <button type="button" role="menuitem" className="menu__item" onClick={() => { setPlus(false); void openExternal("https://drive.google.com/"); }}><Icon n="folder" />Pick from Google Drive</button>
              <div className="menu__sep" />
              <button type="button" role="menuitem" className="menu__item" onClick={() => { setPlus(false); void startMeet(space); }}><Icon n="video" />Start a video call</button>
              <button type="button" role="menuitem" className="menu__item" onClick={() => { setPlus(false); void openExternal("https://calendar.google.com/calendar/r/eventedit"); }}><Icon n="calendar" />Create a calendar invite</button>
            </Menu>
          )}
          <button type="button" className="iconbtn" aria-label="Attach a file" title="Attach" onClick={() => fileRef.current?.click()}>
            <Icon n="clip" />
          </button>
          <input
            ref={fileRef}
            type="file"
            hidden
            onChange={(e) => {
              const f = e.target.files?.[0];
              e.target.value = "";
              attach(f);
            }}
          />
          <button type="button" className="iconbtn" aria-label="Emoji" title="Emoji" onClick={(e) => { const r = e.currentTarget.getBoundingClientRect(); setPicker({ x: r.left + r.width / 2, y: r.top - 300 }); }}>
            <Icon n="smile" />
          </button>
          <button type="button" className="iconbtn" aria-label="Mention someone" title="Mention" onClick={() => insertAt("@")}>
            <Icon n="at" />
          </button>
          <button type="button" className={`composer__send${ready ? " composer__send--ready" : ""}${fly ? " composer__send--fly" : ""}`} aria-label="Send" title="Send (Enter)" disabled={!ready && !fly} onClick={submit}>
            <Icon n="send" />
          </button>
        </div>
      </div>
      <div className="composer__hint">{text.includes("\n") || text.length > 60 ? <><b>Enter</b> to send, <b>Shift+Enter</b> for a new line</> : ""}</div>
      {pick.mounted && anchor && (
        <EmojiPicker at={anchor} exiting={pick.exiting} onClose={() => setPicker(null)} onPick={(u) => { setPicker(null); rememberEmoji(u); insertAt(u); }} />
      )}
    </div>
  );
}
