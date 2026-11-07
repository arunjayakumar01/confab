import { IncomingHuddle } from "./components/HuddleBar";
import { useEffect, useRef } from "react";
import type { Space } from "./bridge-types";
import { boot, closeFeed, closeSearch, goBack, goForward, openFeed, setPanel, stepSpace, toggleThreadPanel, ui, zoom } from "./actions";
import { Composer } from "./components/Composer";
import { FeedView } from "./components/FeedView";
import { Header } from "./components/Header";
import { MembersPanel } from "./components/MembersPanel";
import { MessageList } from "./components/MessageList";
import { DeleteDialog, DropZone, Shortcuts, Toast, Viewer } from "./components/Overlays";
import { AddMembersDialog, FilesPanel, PinnedPanel } from "./components/SpacePanels";
import { Preferences } from "./components/Preferences";
import { QuickSwitcher } from "./components/QuickSwitcher";
import { Rail } from "./components/Rail";
import { SearchView } from "./components/SearchView";
import { Sidebar } from "./components/Sidebar";
import { Loader, SignIn } from "./components/SignIn";
import { ThreadPanel } from "./components/ThreadPanel";
import { TopBar } from "./components/TopBar";
import { TypingStrip } from "./components/TypingStrip";
import { useExit } from "./lib/motion";
import { spaceLabel, useStore } from "./store";

function useTheme() {
  const prefs = useStore((s) => s.prefs);
  const shell = useStore((s) => s.shell);
  // Hidden documents (occluded window, background tab) freeze CSS animations at frame 0, which leaves every
  // fade-in at opacity 0 until the window shows again; while hidden, enter animations complete at once instead.
  useEffect(() => {
    const root = document.documentElement;
    const sync = () => root.toggleAttribute("data-hidden", document.hidden);
    sync();
    document.addEventListener("visibilitychange", sync);
    return () => document.removeEventListener("visibilitychange", sync);
  }, []);
  useEffect(() => {
    const root = document.documentElement;
    if (prefs.theme === "system") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", prefs.theme);
    root.setAttribute("data-reduce-transparency", String(prefs.reduceTransparency));
    root.setAttribute("data-reduce-motion", String(prefs.reduceMotion));
    root.setAttribute("data-shell", shell);
    root.style.setProperty("--sidebar-w", `${prefs.sidebarW}px`);
  }, [prefs, shell]);
}

function useHotkeys() {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      const s = useStore.getState();
      const k = e.key.toLowerCase();
      if (mod && e.shiftKey && k === "a") { e.preventDefault(); openFeed("unreads"); return; }
      if (mod && e.shiftKey && k === "s") { e.preventDefault(); openFeed("saved"); return; }
      if (mod && e.shiftKey && k === "t") { e.preventDefault(); openFeed("todos"); return; }
      if (mod && k === "k") { e.preventDefault(); ui({ switcher: !s.ui.switcher }); return; }
      if (mod && k === "n") { e.preventDefault(); ui({ composeOpen: !s.ui.composeOpen }); return; }
      if (mod && k === "f") { e.preventDefault(); ui({ focusSearch: Date.now() }); return; }
      if (mod && e.key === "/") { e.preventDefault(); ui({ shortcutsOpen: !s.ui.shortcutsOpen }); return; }
      if (mod && e.key === ".") { e.preventDefault(); toggleThreadPanel(); return; }
      if (mod && e.key === "[") { e.preventDefault(); goBack(); return; }
      if (mod && e.key === "]") { e.preventDefault(); goForward(); return; }
      if (mod && (e.key === "=" || e.key === "+")) { e.preventDefault(); zoom(0.1); return; }
      if (mod && e.key === "-") { e.preventDefault(); zoom(-0.1); return; }
      if (mod && e.key === "0") { e.preventDefault(); zoom(0); return; }
      if (e.altKey && (e.key === "ArrowUp" || e.key === "ArrowDown")) {
        e.preventDefault();
        stepSpace(e.key === "ArrowDown" ? 1 : -1, e.shiftKey);
        return;
      }
      if (e.key === "Escape") {
        if (s.ui.viewer) ui({ viewer: undefined });
        else if (s.ui.deleting) ui({ deleting: undefined });
        else if (s.ui.addMembersOpen) ui({ addMembersOpen: false });
        else if (s.ui.spaceMenu) ui({ spaceMenu: false });
        else if (s.ui.shortcutsOpen) ui({ shortcutsOpen: false });
        else if (s.ui.switcher) ui({ switcher: false });
        else if (s.ui.composeOpen) ui({ composeOpen: false });
        else if (s.ui.prefsOpen) ui({ prefsOpen: false });
        else if (s.ui.browseOpen) ui({ browseOpen: false });
        else if (s.ui.editing) ui({ editing: undefined });
        else if (s.ui.search.open) closeSearch();
        else if (s.calendar.editing !== undefined) useStore.setState((st) => ({ calendar: { ...st.calendar, editing: undefined } }));
        else if (s.calendar.selected) useStore.setState((st) => ({ calendar: { ...st.calendar, selected: undefined } }));
        else if (s.notes.editing) useStore.setState((st) => ({ notes: { ...st.notes, editing: undefined } }));
        else if (s.ui.feed) closeFeed();
        else if (s.ui.panel !== "none") setPanel("none");
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
}

export default function App() {
  useTheme();
  useHotkeys();
  useEffect(() => { void boot(); }, []);

  const status = useStore((s) => s.auth.status);
  const spaces = useStore((s) => s.spaces);
  if (status === "loading") return <Loader label="Starting up…" />;
  if (status === "out") return <SignIn />;
  if (spaces.length === 0) return <Loader label="Loading your spaces…" />;
  return <Workspace />;
}

function Workspace() {
  const activeName = useStore((s) => s.ui.activeSpace);
  const space = useStore((s) => s.spaces.find((x) => x.name === s.ui.activeSpace));
  const members = useStore((s) => (activeName ? s.members[activeName] : undefined));
  const me = useStore((s) => s.auth.user?.name);
  const searchOpen = useStore((s) => s.ui.search.open);
  const feed = useStore((s) => s.ui.feed);
  const switcher = useStore((s) => s.ui.switcher);
  const prefsOpen = useStore((s) => s.ui.prefsOpen);
  const shortcutsOpen = useStore((s) => s.ui.shortcutsOpen);
  const deleting = useStore((s) => s.ui.deleting);
  const dl = useExit(!!deleting, 140);
  const addOpen = useStore((s) => s.ui.addMembersOpen);
  const am = useExit(addOpen, 140);
  const sw = useExit(switcher, 140);
  const pf = useExit(prefsOpen, 140);
  const sc = useExit(shortcutsOpen, 140);
  const label = space ? spaceLabel(space, members, me) : "";
  const placeholder = space ? (space.spaceType === "SPACE" ? `Message #${label}` : `Message ${label}`) : "Message";
  return (
    <div className="app">
      <TopBar />
      <div className="app__body">
        <Rail />
        <Sidebar />
        <main className="main" aria-label={space ? label : "Messages"}>
          {searchOpen ? (
            <SearchView />
          ) : feed ? (
            <FeedView key={feed} feed={feed} />
          ) : space ? (
            <>
              <Header space={space} />
              <MessageList key={space.name} space={space} />
              <TypingStrip space={space.name} />
              <Composer space={space.name} placeholder={placeholder} autoFocus />
            </>
          ) : (
            <div className="list__empty"><div><b>Pick a space</b>or press ⌘K.</div></div>
          )}
        </main>
        <PanelSlot space={space} />
      </div>
      {sw.mounted && <QuickSwitcher exiting={sw.exiting} />}
      {pf.mounted && <Preferences exiting={pf.exiting} />}
      {sc.mounted && <Shortcuts exiting={sc.exiting} />}
      {dl.mounted && <DeleteDialog exiting={dl.exiting} />}
      {am.mounted && space && <AddMembersDialog space={space} exiting={am.exiting} />}
      <Viewer />
      <IncomingHuddle />
      <Toast />
      <DropZone />
    </div>
  );
}

/** The right-hand panel slot: thread or members, kept mounted while it slides out. */
function PanelSlot({ space }: { space?: Space }) {
  const panel = useStore((s) => s.ui.panel);
  const thread = useStore((s) => s.ui.thread);
  const kind = panel === "thread" && thread ? "thread" : (panel === "members" || panel === "files" || panel === "pinned") && space ? panel : "none";
  const { mounted, exiting } = useExit(kind !== "none", 200);
  const last = useRef<{ kind: "thread" | "members" | "files" | "pinned"; thread?: string; space?: Space } | null>(null);
  if (kind !== "none") last.current = { kind, thread, space };
  if (!mounted || !last.current) return null;
  const l = last.current;
  if (l.kind === "thread" && l.thread) return <ThreadPanel thread={l.thread} exiting={exiting} />;
  if (l.kind === "members" && l.space) return <MembersPanel space={l.space} exiting={exiting} />;
  if (l.kind === "files" && l.space) return <FilesPanel key={l.space.name} space={l.space} exiting={exiting} />;
  if (l.kind === "pinned" && l.space) return <PinnedPanel key={l.space.name} space={l.space} exiting={exiting} />;
  return null;
}
