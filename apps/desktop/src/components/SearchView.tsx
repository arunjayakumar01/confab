import { closeSearch, openSpace, openThread } from "../actions";
import { plainText } from "../lib/markdown";
import { stagger } from "../lib/motion";
import { dateShort, parseTime } from "../lib/time";
import { bodyOf, nameMap, spaceLabel, useStore } from "../store";
import { Avatar } from "./Avatar";
import { Icon } from "./icons";

export function SearchView() {
  const { query, results, loading } = useStore((s) => s.ui.search);
  const spaces = useStore((s) => s.spaces);
  const members = useStore((s) => s.members);
  const me = useStore((s) => s.auth.user?.name);
  const q = query.trim();
  const mark = (text: string) => {
    if (!q) return text;
    const parts = text.split(new RegExp(`(${q.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")})`, "ig"));
    return parts.map((p, i) => (p.toLowerCase() === q.toLowerCase() ? <mark key={i}>{p}</mark> : p));
  };
  return (
    <>
      <div className="header glass-strong">
        <div className="header__name">Search</div>
        <div className="header__meta"><span className="header__desc">"{query}"</span></div>
        <div className="header__spacer" />
        <button type="button" className="iconbtn" aria-label="Close search" onClick={closeSearch}><Icon n="x" /></button>
      </div>
      <div className="results" key={query}>
        <div className="results__head">{loading ? "Searching…" : `${results.length} ${results.length === 1 ? "result" : "results"}`}</div>
        {results.map((m, i) => {
          const space = m.name.split("/").slice(0, 2).join("/");
          const sp = spaces.find((s) => s.name === space);
          const names = nameMap(members[space]);
          return (
            <button
              key={m.name}
              type="button"
              className="result"
              style={stagger(i)}
              onClick={() => {
                openSpace(space);
                if (m.threadReply && m.thread?.name) openThread(m.thread.name);
              }}
            >
              <div className="result__meta">
                <Avatar user={m.sender} size={18} />
                <b>{m.sender?.displayName ?? "Someone"}</b>
                <span className="result__space">{sp ? (sp.spaceType === "SPACE" ? `#${spaceLabel(sp, members[space], me)}` : spaceLabel(sp, members[space], me)) : space}</span>
                <span>{dateShort(parseTime(m.createTime))}</span>
              </div>
              <div className="result__text">{mark(plainText(bodyOf(m), (u) => names[u]?.displayName ?? u))}</div>
            </button>
          );
        })}
        {!loading && results.length === 0 && <div className="dialog__empty">No messages match "{query}".</div>}
      </div>
    </>
  );
}
