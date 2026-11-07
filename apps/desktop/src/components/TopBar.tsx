import { useEffect, useRef, useState } from "react";
import { closeSearch, goBack, goForward, openExternal, openFeed, runSearch, selectEvent, upcoming } from "../actions";
import { meetLinkOf, startOf } from "../lib/cal";
import { useStore } from "../store";
import { Icon } from "./icons";

export function TopBar() {
  const shell = useStore((s) => s.shell);
  const relay = useStore((s) => s.relay);
  const focusTick = useStore((s) => s.ui.focusSearch);
  const canBack = useStore((s) => s.ui.history.length > 0);
  const canForward = useStore((s) => s.ui.future.length > 0);
  const searchOpen = useStore((s) => s.ui.search.open);
  const nextId = useStore((s) => upcoming(s)?.id);
  const next = useStore((s) => (nextId ? Object.values(s.calendar.events).find((e) => e.id === nextId) : undefined));
  const [, tick] = useState(0);
  useEffect(() => { const t = window.setInterval(() => tick((n) => n + 1), 30_000); return () => window.clearInterval(t); }, []);
  const mins = next ? Math.round((startOf(next).getTime() - Date.now()) / 60000) : 0;
  const [q, setQ] = useState("");
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (focusTick) {
      ref.current?.focus();
      ref.current?.select();
    }
  }, [focusTick]);
  useEffect(() => {
    if (!searchOpen) setQ("");
  }, [searchOpen]);
  const drag = shell === "desktop" ? { "data-tauri-drag-region": true } : {};
  return (
    <header className="topbar" {...drag}>
      <div className="topbar__nav">
        <button type="button" className="iconbtn" aria-label="Back (⌘[)" title="Back ⌘[" disabled={!canBack} onClick={goBack}>
          <Icon n="left" />
        </button>
        <button type="button" className="iconbtn" aria-label="Forward (⌘])" title="Forward ⌘]" disabled={!canForward} onClick={goForward}>
          <Icon n="right" />
        </button>
      </div>
      <div className="topbar__spacer" {...drag} />
      <form
        className="topbar__search"
        role="search"
        onSubmit={(e) => {
          e.preventDefault();
          void runSearch(q);
        }}
      >
        <Icon n="search" />
        <input
          ref={ref}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder="Search messages"
          aria-label="Search messages"
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              e.stopPropagation();
              setQ("");
              closeSearch();
              ref.current?.blur();
            }
          }}
        />
        <span className="kbd">⌘F</span>
      </form>
      <div className="topbar__spacer" {...drag} />
      {next && (
        <button type="button" className={`topbar__next${mins <= 5 ? " topbar__next--soon" : ""}`} title={next.summary ?? "Event"} onClick={() => { const link = meetLinkOf(next); if (link && mins <= 10) void openExternal(link); else { openFeed("calendar"); selectEvent(`${next.calendarId ?? "primary"}:${next.id}`); } }}>
          <Icon n={meetLinkOf(next) ? "video" : "calendar"} />
          <span className="topbar__nexttitle">{next.summary ?? "Event"}</span>
          <span className="topbar__nextwhen">{mins <= 0 ? "now" : mins < 60 ? `in ${mins} min` : `at ${startOf(next).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}`}</span>
          {meetLinkOf(next) && mins <= 10 && <span className="topbar__join">Join</span>}
        </button>
      )}
      <div className="topbar__right">
        <span
          className={`topbar__relay topbar__relay--${relay}`}
          role="status"
          aria-label={relay === "open" ? "Relay connected" : relay === "connecting" ? "Relay connecting" : "Relay offline"}
          title={relay === "open" ? "Realtime connected" : relay === "connecting" ? "Connecting to the relay…" : "Realtime offline, reconnecting"}
        />
      </div>
    </header>
  );
}
