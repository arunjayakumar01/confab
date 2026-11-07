import { useMemo } from "react";
import { setPanel } from "../actions";
import { nameMap, spaceLabel, useStore } from "../store";
import { Composer } from "./Composer";
import { Icon } from "./icons";
import { positions } from "./MessageList";
import { MessageRow } from "./MessageRow";
import { TypingStrip } from "./TypingStrip";

/** note: replies are not virtualized; threads of a few hundred messages are fine, thousands are not. */
export function ThreadPanel({ thread, exiting }: { thread: string; exiting?: boolean }) {
  const space = thread.split("/").slice(0, 2).join("/");
  const sp = useStore((s) => s.spaces.find((x) => x.name === space));
  const members = useStore((s) => s.members[space]);
  const me = useStore((s) => s.auth.user?.name);
  const spaceItems = useStore((s) => s.messages[space]?.items);
  const tpage = useStore((s) => s.threads[thread]);
  const names = useMemo(() => nameMap(members), [members]);
  const { parent, replies } = useMemo(() => {
    const seen = new Set<string>();
    const all = [...(tpage?.items ?? []), ...(spaceItems ?? []).filter((m) => m.thread?.name === thread)]
      .filter((m) => (seen.has(m.name) ? false : (seen.add(m.name), true)))
      .sort((a, b) => ((a.createTime ?? "") < (b.createTime ?? "") ? -1 : 1));
    const parent = all.find((m) => !m.threadReply) ?? all[0];
    return { parent, replies: all.filter((m) => m !== parent) };
  }, [tpage, spaceItems, thread]);
  const label = sp ? spaceLabel(sp, members, me) : space;
  const pos = positions(replies);
  return (
    <aside className={`panel glass${exiting ? " panel--out" : ""}`} aria-label="Thread">
      <div className="panel__head">
        <span className="panel__title">Thread</span>
        <span className="panel__sub">{sp?.spaceType === "SPACE" ? `#${label}` : label}</span>
        <button type="button" className="iconbtn" aria-label="Close thread" onClick={() => setPanel("none")}>
          <Icon n="x" />
        </button>
      </div>
      <div className="panel__scroll" role="list" aria-label="Thread messages">
        {parent && <MessageRow m={parent} collapsed={false} me={me} members={names} inThread />}
        {!parent && <div className="list__top">{tpage?.loading ? "Loading…" : "This thread is empty."}</div>}
        {replies.length > 0 && <div className="panel__count">{replies.length} {replies.length === 1 ? "reply" : "replies"}</div>}
        {replies.map((m, i) => {
          const p = pos[i]!;
          return <MessageRow key={m.name} m={m} collapsed={p === "mid" || p === "last"} pos={p} me={me} members={names} inThread />;
        })}
      </div>
      <TypingStrip space={space} thread={thread} />
      <Composer space={space} thread={thread} placeholder="Reply…" autoFocus />
    </aside>
  );
}
