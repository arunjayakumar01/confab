import { memo, useState, type ReactNode } from "react";
import type { User } from "../bridge-types";
import { openExternal, toast } from "../actions";
import { parseMarkdown, type Block, type Inline } from "../lib/markdown";
import { Icon } from "./icons";

function renderInline(nodes: Inline[], members: Record<string, User>, me?: string): ReactNode[] {
  return nodes.map((n, i) => {
    switch (n.t) {
      case "text":
        return n.v;
      case "br":
        return <br key={i} />;
      case "bold":
        return <strong key={i}>{renderInline(n.c, members, me)}</strong>;
      case "italic":
        return <em key={i}>{renderInline(n.c, members, me)}</em>;
      case "strike":
        return <s key={i}>{renderInline(n.c, members, me)}</s>;
      case "code":
        return <code key={i}>{n.v}</code>;
      case "link":
        // The shell opens links in the system browser; a webview `_blank` goes nowhere on desktop.
        return (
          <a key={i} href={n.href} title={n.href} onClick={(e) => { e.preventDefault(); void openExternal(n.href); }}>
            {n.label}
          </a>
        );
      case "mention": {
        const all = n.user === "users/all";
        const name = all ? "all" : members[n.user]?.displayName ?? n.user.replace("users/", "user ");
        const mine = all || n.user === me;
        return (
          <span key={i} className={`mention${mine ? " mention--me" : ""}`}>
            @{name}
          </span>
        );
      }
    }
  });
}

function CodeBlock({ lang, v }: { lang?: string; v: string }) {
  const copy = () => { void navigator.clipboard?.writeText(v); toast("Code copied"); };
  return (
    <div className="codeblock">
      <div className="codeblock__bar">
        <span>{lang ?? "code"}</span>
        <button type="button" className="codeblock__copy" onClick={copy} aria-label="Copy code"><Icon n="copy" size={12} />Copy</button>
      </div>
      <pre><code>{v}</code></pre>
    </div>
  );
}

function renderBlock(b: Block, i: number, members: Record<string, User>, me?: string): ReactNode {
  switch (b.t) {
    case "p":
      return <p key={i}>{renderInline(b.c, members, me)}</p>;
    case "code":
      return <CodeBlock key={i} lang={b.lang} v={b.v} />;
    case "quote":
      return <blockquote key={i}>{renderInline(b.c, members, me)}</blockquote>;
    case "list": {
      const items = b.items.map((it, j) => <li key={j}>{renderInline(it, members, me)}</li>);
      return b.ordered ? <ol key={i} start={b.start}>{items}</ol> : <ul key={i}>{items}</ul>;
    }
  }
}

const FOLD_LINES = 16;
const FOLD_CHARS = 1400;
/** Long messages fold at a fixed height with a fade; the choice is remembered while the row is mounted. */
export const isLong = (text: string): boolean => text.length > FOLD_CHARS || (text.match(/\n/g)?.length ?? 0) >= FOLD_LINES;

export const MarkdownView = memo(function MarkdownView({ text, members, me, fold }: { text: string; members: Record<string, User>; me?: string; fold?: boolean }) {
  const [open, setOpen] = useState(false);
  const blocks = parseMarkdown(text);
  const folded = !!fold && !open && isLong(text);
  return (
    <>
      <div className={`msg__text${folded ? " msg__text--folded" : ""}`}>{blocks.map((b, i) => renderBlock(b, i, members, me))}</div>
      {fold && isLong(text) && (
        <button type="button" className={`msg__more${folded ? "" : " msg__more--open"}`} onClick={() => setOpen((o) => !o)} aria-expanded={!folded}>
          {folded ? "Show more" : "Show less"}
          <Icon n="down" size={12} />
        </button>
      )}
    </>
  );
});
