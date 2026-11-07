import { create } from "zustand";
import type { Account, CalEvent, CalendarEntry, CustomStatus, LinkPreview, MeetParticipant, Membership, Message, PresenceState, Space, Task, User } from "./bridge-types";
import type { ShellKind } from "./bridge";

export interface MessagePage { items: Message[]; nextPageToken?: string; loaded: boolean; loading: boolean }
export interface TypingEntry { until: number; thread?: string }
export interface Prefs {
  theme: "system" | "light" | "dark"; reduceTransparency: boolean; reduceMotion: boolean; readReceipts: boolean;
  sounds: boolean; zoom: number; sidebarW: number;
  notifications: "all" | "mentions" | "off";
  /** Fetching a page shows that site your address, so previews wait for a yes: per link, per site, or always. */
  linkPreviews: "off" | "ask" | "on";
}
export interface Toast { id: number; text: string }
/** A bookmarked message with enough of a snapshot to list it before its space is loaded. */
export interface SavedItem { name: string; space: string; at: string; text?: string; sender?: User; createTime?: string }
export type Feed = "unreads" | "saved" | "todos" | "people" | "calendar" | "notes";
export type CalView = "day" | "week" | "month" | "agenda";
/** A Keep-style note; the whole list lives as one JSON document in Drive app data. */
export interface Note { id: string; title: string; body: string; color: string; pinned: boolean; updated: string }
export type Panel = "none" | "thread" | "members" | "files" | "pinned";
export type RailView = "home" | "dms" | "activity";

export interface SearchState { open: boolean; query: string; results: Message[]; loading: boolean }

export interface State {
  shell: ShellKind;
  auth: { status: "loading" | "out" | "in"; user?: User; error?: string };
  relay: "connecting" | "open" | "closed";
  spaces: Space[];
  members: Record<string, Membership[]>;
  messages: Record<string, MessagePage>; // by space, ascending createTime
  threads: Record<string, MessagePage>; // by thread name, ascending
  myReactions: Record<string, string[]>; // message name -> unicodes
  unread: Record<string, number>;
  typing: Record<string, Record<string, TypingEntry>>; // space -> user -> entry
  presence: Record<string, { state: PresenceState; custom_status?: CustomStatus }>;
  receipts: Record<string, Record<string, { message: string; at: string }>>; // space -> user
  drafts: Record<string, string>; // space or `${space}|${thread}`
  starred: string[];
  muted: string[];
  saved: SavedItem[];
  recentEmoji: string[];
  holdRead: Record<string, true>; // spaces marked unread by hand: stays until the space is opened again
  tasks: { items: Task[]; loaded: boolean; error?: string };
  accounts: Account[]; // every Google account signed into this Confab; the active one owns the rest of the store
  calendar: {
    calendars: CalendarEntry[]; events: Record<string, CalEvent>; syncTokens: Record<string, string>; hidden: string[];
    loaded: boolean; loading: boolean; error?: string; view: CalView; day: string; selected?: string; editing?: Partial<CalEvent> | null; polledAt?: number;
  };
  notes: { items: Note[]; loaded: boolean; saving: boolean; error?: string; query: string; editing?: string };
  huddles: Record<string, string[]>; // space -> users in a Confab voice huddle, from the relay
  /** The huddle I am in: Meet-backed (Google's own, open in the browser, who is in from the Meet API) or Confab voice. */
  huddle: {
    kind?: "meet" | "voice"; space?: string; code?: string; uri?: string; message?: string; active: boolean; participants: MeetParticipant[];
    starting?: boolean; // the click was taken; Google has not answered yet
    muted: boolean; listenOnly: boolean; peers: Record<string, "connecting" | "connected" | "failed">; speaking: string[]; error?: string;
  };
  pinned: Record<string, string[]>; // space -> message names, local only (Google has no pin API)
  previewDomains: string[]; // sites the user said "always" to
  previews: Record<string, LinkPreview | null | { loading: true } | { error: string }>; // by URL, this session
  prefs: Prefs;
  me: { state: PresenceState; custom?: CustomStatus };
  ringing?: { space: string; code: string; uri: string; message: string; from: string };
  ui: {
    view: RailView;
    activeSpace?: string;
    thread?: string;
    panel: Panel;
    switcher: boolean;
    search: SearchState;
    prefsOpen: boolean;
    browseOpen: boolean;
    composeOpen: boolean;
    statusMenu: boolean;
    titleMenu: boolean;
    dragOver: boolean;
    toast?: Toast;
    editing?: string;
    focusSearch: number;
    history: string[];
    future: string[];
    feed?: Feed;
    unreadAtOpen: number; // the unread count the active space had when it was opened: where the "New" line goes
    jumpTo?: { message: string; at: number };
    viewer?: { src: string; name: string; mime?: string; open?: string; back?: { src: string; name: string; mime?: string } };
    deleting?: string; // message name awaiting the delete confirmation
    addMembersOpen: boolean;
    spaceMenu: boolean;
    todosSpace?: string; // Todos filtered to tasks made from this space
    pickFile: number; // tick: the composer opens its file picker
    addingFrom?: string; // the account to return to if adding another one is cancelled
    shortcutsOpen: boolean;
  };
  set: (fn: (s: State) => Partial<State> | void) => void;
}

const PREFS_KEY = "prism.prefs";
export const todayKey = (d = new Date()): string => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
const STAR_KEY = "prism.starred";
const MUTE_KEY = "prism.muted";
const SAVED_KEY = "prism.saved";
const EMOJI_KEY = "prism.emoji";
const DOMAINS_KEY = "prism.previewDomains";
const PIN_KEY = "prism.pinned";
const ME_KEY = "prism.me";

function load<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    if (!raw) return fallback;
    const v = JSON.parse(raw) as T;
    // Arrays come back whole; objects pick up any key a newer build added to the fallback.
    return Array.isArray(fallback) ? (Array.isArray(v) ? v : fallback) : { ...fallback, ...v };
  } catch {
    return fallback;
  }
}

export const useStore = create<State>()((set) => ({
  shell: "browser",
  auth: { status: "loading" },
  relay: "closed",
  spaces: [],
  members: {},
  messages: {},
  threads: {},
  myReactions: {},
  unread: {},
  typing: {},
  presence: {},
  receipts: {},
  drafts: {},
  starred: load<string[]>(STAR_KEY, []),
  muted: load<string[]>(MUTE_KEY, []),
  saved: load<SavedItem[]>(SAVED_KEY, []),
  recentEmoji: load<string[]>(EMOJI_KEY, []),
  holdRead: {},
  tasks: { items: [], loaded: false },
  accounts: [],
  calendar: { calendars: [], events: {}, syncTokens: {}, hidden: load<string[]>("prism.calHidden", []), loaded: false, loading: false, view: "week", day: todayKey() },
  notes: { items: [], loaded: false, saving: false, query: "" },
  huddles: {},
  huddle: { active: false, participants: [], muted: false, listenOnly: false, peers: {}, speaking: [] },
  pinned: load<Record<string, string[]>>(PIN_KEY, {}),
  previewDomains: load<string[]>(DOMAINS_KEY, []),
  previews: {},
  prefs: load<Prefs>(PREFS_KEY, { theme: "system", reduceTransparency: false, reduceMotion: false, readReceipts: true, sounds: true, zoom: 1, sidebarW: 260, notifications: "all", linkPreviews: "ask" }),
  me: load<State["me"]>(ME_KEY, { state: "active" }),
  ui: {
    view: "home",
    panel: "none",
    switcher: false,
    search: { open: false, query: "", results: [], loading: false },
    prefsOpen: false,
    browseOpen: false,
    composeOpen: false,
    statusMenu: false,
    titleMenu: false,
    dragOver: false,
    focusSearch: 0,
    history: [],
    future: [],
    unreadAtOpen: 0,
    shortcutsOpen: false,
    addMembersOpen: false,
    spaceMenu: false,
    pickFile: 0,
  },
  set: (fn) =>
    set((s) => {
      const next = fn(s);
      return next ? { ...next } : {};
    }),
}));

useStore.subscribe((s, prev) => {
  if (s.prefs !== prev.prefs) localStorage.setItem(PREFS_KEY, JSON.stringify(s.prefs));
  if (s.starred !== prev.starred) localStorage.setItem(STAR_KEY, JSON.stringify(s.starred));
  if (s.muted !== prev.muted) localStorage.setItem(MUTE_KEY, JSON.stringify(s.muted));
  if (s.saved !== prev.saved) localStorage.setItem(SAVED_KEY, JSON.stringify(s.saved));
  if (s.recentEmoji !== prev.recentEmoji) localStorage.setItem(EMOJI_KEY, JSON.stringify(s.recentEmoji));
  if (s.previewDomains !== prev.previewDomains) localStorage.setItem(DOMAINS_KEY, JSON.stringify(s.previewDomains));
  if (s.pinned !== prev.pinned) localStorage.setItem(PIN_KEY, JSON.stringify(s.pinned));
  if (s.calendar.hidden !== prev.calendar.hidden) localStorage.setItem("prism.calHidden", JSON.stringify(s.calendar.hidden));
  if (s.me !== prev.me) localStorage.setItem(ME_KEY, JSON.stringify(s.me));
});

// ---- pure helpers over message lists ----
/** The body to render or edit: Google puts the formatting markup, mentions and links in `formattedText`; `text` is plain. */
export const bodyOf = (m: Pick<Message, "text" | "formattedText">): string => m.formattedText ?? m.text ?? "";
/** True when the message mentions `me` or everyone, by markup or by Google's annotations (UI-typed mentions carry both). */
export function mentionsMe(m: Message, me: string | undefined): boolean {
  if (!me) return false;
  const b = bodyOf(m);
  if (b.includes(`<${me}>`) || b.includes("<users/all>") || b.includes(`data-user="${me}"`)) return true;
  return (m.annotations ?? []).some((a) => a.userMention?.user?.name === me);
}
export const byTime = (a: Message, b: Message) =>
  (a.createTime ?? "") < (b.createTime ?? "") ? -1 : (a.createTime ?? "") > (b.createTime ?? "") ? 1 : 0;

/** Inserts or replaces by name (or by clientAssignedMessageId for optimistic→server swaps); keeps ascending order. */
export function upsert(list: Message[], msg: Message): Message[] {
  const cid = msg.clientAssignedMessageId;
  const i = list.findIndex((m) => m.name === msg.name || (cid && (m.clientAssignedMessageId === cid || m.name.endsWith("/" + cid))));
  const next = [...list];
  if (i >= 0) next[i] = { ...next[i], ...msg };
  else next.push(msg);
  return next.sort(byTime);
}
export function remove(list: Message[], name: string): Message[] {
  return list.filter((m) => m.name !== name);
}
export function patch(list: Message[], name: string, fn: (m: Message) => Message): Message[] {
  return list.map((m) => (m.name === name ? fn(m) : m));
}
export function emptyPage(): MessagePage {
  return { items: [], loaded: false, loading: false };
}
export const draftKey = (space: string, thread?: string) => (thread ? `${space}|${thread}` : space);

// ---- selectors ----
export function spaceLabel(space: Space, members: Membership[] | undefined, me: string | undefined): string {
  if (space.displayName) return space.displayName;
  const others = (members ?? []).map((m) => m.member).filter((u): u is User => !!u && u.name !== me);
  if (others.length) return others.map((u) => u.displayName ?? (u.type === "BOT" ? "App" : u.name)).join(", ");
  return space.spaceType === "DIRECT_MESSAGE" ? "Direct message" : "Group chat";
}
export function dmPeer(space: Space, members: Membership[] | undefined, me: string | undefined): User | undefined {
  if (space.spaceType !== "DIRECT_MESSAGE") return undefined;
  return (members ?? []).map((m) => m.member).find((u): u is User => !!u && u.name !== me);
}
export function nameMap(members: Membership[] | undefined): Record<string, User> {
  const out: Record<string, User> = {};
  for (const m of members ?? []) if (m.member) out[m.member.name] = m.member;
  return out;
}
export interface ThreadInfo { count: number; repliers: User[]; last: string }
export function threadInfo(items: Message[]): Record<string, ThreadInfo> {
  const out: Record<string, ThreadInfo> = {};
  for (const m of items) {
    if (!m.threadReply || !m.thread?.name) continue;
    const t = (out[m.thread.name] ??= { count: 0, repliers: [], last: "" });
    t.count += 1;
    if (m.sender && !t.repliers.some((u) => u.name === m.sender!.name)) t.repliers.push(m.sender);
    if ((m.createTime ?? "") > t.last) t.last = m.createTime ?? "";
  }
  return out;
}
