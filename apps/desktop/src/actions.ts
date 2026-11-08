import { huddleOf, isHuddleStart } from "./lib/meet";
// Every side effect lives here: bridge calls, realtime frames, navigation. Components only read the store.
import { detectShell, getBridge } from "./bridge";
import type { Attachment, Bridge, CalEvent, ChatEvent, CustomStatus, DriveItem, DriveWhere, EmojiReactionSummary, EventInput, Message, PresenceState, ServerMsg, ShellEvent, Space, Task, User } from "./bridge-types";
import { isFolder, isGoogleFile, targetId } from "./lib/drive";
import { previewKind } from "./lib/files";
import { addDays, endOf, isAllDay, nextUp, startOf, startOfWeek } from "./lib/cal";
import { plainText } from "./lib/markdown";
import { driveFromText } from "./lib/files";
import { clientIdOf, uuid } from "./lib/ids";
import { Huddle, type HuddleSignalPayload } from "./lib/huddle";
import { chime, unlockAudio } from "./lib/sound";
import { bodyOf, byTime, dmPeer, draftKey, emptyPage, mentionsMe, patch, remove, spaceLabel, todayKey, upsert, useStore, type CalView, type DriveFolder, type Feed, type Note, type Prefs, type State } from "./store";

let bridge: Bridge;
const S = useStore;
const LAST_SPACE_KEY = "prism.lastSpace";
const me = () => S.getState().auth.user?.name;

export async function boot(): Promise<void> {
  const shell = detectShell();
  S.setState({ shell });
  bridge = await getBridge();
  bridge.onMessage(handleFrame);
  window.setInterval(pruneTyping, 1000);
  bridge.onFileDrop?.((paths) => {
    if (S.getState().ui.feed === "drive") { void driveUploadPaths(paths); return; }
    const { activeSpace, thread, panel } = S.getState().ui;
    if (activeSpace) for (const p of paths) void uploadPathAndSend(activeSpace, p, panel === "thread" ? thread : undefined);
  });
  bridge.onFileDrag?.((over) => ui({ dragOver: over }));
  if (shell !== "desktop") wireHtmlDrop();
  // Dock badge: total unread across unmuted spaces. The shell ignores it where the platform has no badge.
  if (bridge.setBadge) {
    let last = -1;
    S.subscribe((s) => {
      const n = unreadTotal(s);
      if (n !== last) { last = n; bridge.setBadge!(n); }
    });
  }
  unlockAudio();
  if (bridge.setZoom && S.getState().prefs.zoom !== 1) void bridge.setZoom(S.getState().prefs.zoom);
  // Browsers only grant Notification permission from a gesture; the first click asks once.
  if (shell !== "desktop" && typeof Notification !== "undefined" && Notification.permission === "default") {
    const ask = () => { void Notification.requestPermission(); window.removeEventListener("pointerdown", ask, true); };
    window.addEventListener("pointerdown", ask, true);
  }
  try {
    const st = await bridge.authStatus();
    if (st.signedIn) {
      S.setState({ auth: { status: "in", user: st.user } });
      await loadWorkspace();
    } else S.setState({ auth: { status: "out" } });
  } catch (e) {
    S.setState({ auth: { status: "out", error: String(e) } });
  }
}

export async function signIn(): Promise<void> {
  S.setState({ auth: { status: "loading" } });
  try {
    const st = await bridge.authStart();
    if (st.signedIn) {
      S.setState((s) => ({ auth: { status: "in", user: st.user }, ui: { ...s.ui, addingFrom: undefined } }));
      await loadWorkspace();
    } else S.setState({ auth: { status: "out", error: "Sign-in did not complete" } });
  } catch (e) {
    S.setState({ auth: { status: "out", error: String(e) } });
  }
}

export async function signOut(): Promise<void> {
  leaveHuddle();
  try { await bridge.signOut(); } catch { /* best effort */ }
  localStorage.removeItem(LAST_SPACE_KEY);
  S.setState({
    auth: { status: "out" }, spaces: [], members: {}, messages: {}, threads: {}, unread: {}, typing: {}, presence: {}, receipts: {},
    myReactions: {}, relay: "closed", holdRead: {},
    ui: {
      ...S.getState().ui, activeSpace: undefined, thread: undefined, panel: "none", prefsOpen: false, switcher: false, composeOpen: false, titleMenu: false, statusMenu: false,
      history: [], future: [], feed: undefined, jumpTo: undefined, viewer: undefined, deleting: undefined, shortcutsOpen: false,
    },
  });
}

/** Unread across spaces that are not muted: the dock badge and the rail dots read this. */
export const unreadTotal = (s: State): number => Object.entries(s.unread).reduce((a, [space, n]) => a + (s.muted.includes(space) ? 0 : n), 0);

/** Browser and mock shells: HTML drag events on the window stand in for the Tauri drop hook. */
function wireHtmlDrop(): void {
  let depth = 0;
  const hasFiles = (e: DragEvent) => Array.from(e.dataTransfer?.types ?? []).includes("Files");
  window.addEventListener("dragenter", (e) => { if (!hasFiles(e)) return; e.preventDefault(); depth += 1; ui({ dragOver: true }); });
  window.addEventListener("dragover", (e) => { if (hasFiles(e)) e.preventDefault(); });
  window.addEventListener("dragleave", (e) => { if (!hasFiles(e)) return; depth = Math.max(0, depth - 1); if (depth === 0) ui({ dragOver: false }); });
  window.addEventListener("drop", (e) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    depth = 0;
    ui({ dragOver: false });
    if (S.getState().ui.feed === "drive") { void driveUploadFiles(Array.from(e.dataTransfer?.files ?? [])); return; }
    const { activeSpace, thread, panel } = S.getState().ui;
    if (!activeSpace) return;
    for (const f of Array.from(e.dataTransfer?.files ?? [])) void uploadAndSend(activeSpace, f, panel === "thread" ? thread : undefined).catch((err) => toast(String(err)));
  });
}

let toastSeq = 0;
let toastTimer = 0;
/** A short notice at the bottom of the window; replaces the previous one. */
export function toast(text: string, ms = 2400): void {
  const id = ++toastSeq;
  ui({ toast: { id, text } });
  window.clearTimeout(toastTimer);
  toastTimer = window.setTimeout(() => S.setState((s) => (s.ui.toast?.id === id ? { ui: { ...s.ui, toast: undefined } } : {})), ms);
}

/** Google links carry the signed-in account (`authuser`) and, on desktop, go to the Chrome profile signed in as it. */
export async function openExternal(url: string): Promise<void> {
  const email = S.getState().auth.user?.email;
  let target = url;
  let account: string | undefined;
  try {
    const u = new URL(url);
    if (email && /(^|\.)google\.com$/i.test(u.hostname)) {
      if (!u.searchParams.has("authuser")) u.searchParams.set("authuser", email);
      target = u.toString();
      account = email;
    }
  } catch { /* not a URL we can shape; open as given */ }
  try { await bridge.openUrl(target, account); } catch (e) { toast(`Could not open link: ${String(e)}`); }
}

/** Uploaded files on desktop come through the prism-media scheme, which only the shell can save to disk. */
export async function saveAttachment(url: string, name: string): Promise<void> {
  if (!bridge.saveAttachment) {
    const a = document.createElement("a");
    a.href = url;
    a.download = name;
    a.click();
    return;
  }
  try {
    const path = await bridge.saveAttachment(url, name);
    toast(`Saved to ${path.split("/").slice(-2).join("/")}`);
  } catch (e) {
    toast(`Could not save: ${String(e)}`);
  }
}

/** Sidebar order: starred, channels A-Z, DMs by activity, groups by activity. Pure; memoize at call sites. */
export function sidebarOrder(spaces: Space[], members: State["members"], starred: string[], me: string | undefined): Space[] {
  const label = (sp: Space) => spaceLabel(sp, members[sp.name], me).toLowerCase();
  const channels = spaces.filter((x) => x.spaceType === "SPACE" || !x.spaceType).sort((a, b) => label(a).localeCompare(label(b)));
  const dms = spaces.filter((x) => x.spaceType === "DIRECT_MESSAGE").sort((a, b) => (b.lastActiveTime ?? "").localeCompare(a.lastActiveTime ?? ""));
  const groups = spaces.filter((x) => x.spaceType === "GROUP_CHAT").sort((a, b) => (b.lastActiveTime ?? "").localeCompare(a.lastActiveTime ?? ""));
  const stars = spaces.filter((x) => starred.includes(x.name));
  const rest = [...channels, ...dms, ...groups].filter((x) => !starred.includes(x.name));
  return [...stars, ...rest];
}
const orderOf = (s: State) => sidebarOrder(s.spaces, s.members, s.starred, s.auth.user?.name);

async function loadWorkspace(): Promise<void> {
  void loadAccounts();
  const spaces = await bridge.listSpaces();
  S.setState({ spaces });
  // DM and group labels come from the member list, so fetch those up front.
  await Promise.all(spaces.filter((s) => s.spaceType !== "SPACE").map((s) => ensureMembers(s.name)));
  subscribeRelay();
  void resolveDmNames(spaces);
  const last = localStorage.getItem(LAST_SPACE_KEY);
  const first = spaces.find((s) => s.name === last) ?? orderOf(S.getState())[0];
  if (first) openSpace(first.name);
  void syncPresenceFromGoogle();
  // The next-meeting chip and reminders need the calendar even when its view is closed: load once, then every 3 min.
  void loadCalendar().then(() => { window.setInterval(() => { if (!calPoll) void pollCalendar(); }, 180_000); });
}

/** Google leaves displayName empty on many memberships (apps, some people); the last message's sender has it. */
async function resolveDmNames(spaces: Space[]): Promise<void> {
  const me = S.getState().auth.user?.name;
  const bare = spaces.filter((sp) => {
    if (sp.spaceType !== "DIRECT_MESSAGE") return false;
    const peer = dmPeer(sp, S.getState().members[sp.name], me);
    return !peer?.displayName;
  });
  await Promise.all(bare.map(async (sp) => {
    try {
      const r = await bridge.listMessages(sp.name, { pageSize: 5 });
      const sender = r.items.map((m) => m.sender).find((u) => u && u.name !== me && u.displayName);
      if (!sender) return;
      S.setState((st) => {
        const list = st.members[sp.name] ?? [];
        const has = list.some((m) => m.member?.name === sender.name);
        const members = has
          ? list.map((m) => (m.member?.name === sender.name ? { ...m, member: { ...m.member, ...sender } } : m))
          : [...list, { name: `${sp.name}/members/${sender.name.replace("users/", "")}`, member: sender }];
        return { members: { ...st.members, [sp.name]: members } };
      });
    } catch { /* name stays bare */ }
  }));
}

function subscribeRelay(): void {
  const s = S.getState();
  if (s.relay !== "open" || !s.spaces.length) return;
  bridge.relaySend({ type: "subscribe", spaces: s.spaces.map((x) => x.name) });
  bridge.relaySend({ type: "presence", state: s.me.state, custom_status: s.me.custom });
  if (voice) bridge.relaySend({ type: "huddle_join", space: voice.space });
}

export function openSpace(name: string, opts: { keepPanel?: boolean; nav?: "back" | "forward" } = {}): void {
  const s = S.getState();
  if (!s.spaces.some((x) => x.name === name)) return;
  const cur = s.ui.activeSpace;
  const moved = !!cur && cur !== name;
  // Browser-style history: a fresh jump pushes the current space and drops the forward stack.
  let history = s.ui.history;
  let future = s.ui.future;
  if (moved) {
    if (opts.nav === "back") { history = history.slice(0, -1); future = [cur, ...future].slice(0, 30); }
    else if (opts.nav === "forward") { history = [...history.slice(-30), cur]; future = future.slice(1); }
    else { history = [...history.slice(-30), cur]; future = []; }
  }
  // A space marked unread by hand stays that way until it is opened from elsewhere; re-clicking the open one keeps the hold.
  const held = !!s.holdRead[name] && !moved && !s.ui.feed;
  const holdRead = { ...s.holdRead };
  if (!held) delete holdRead[name];
  S.setState({
    unread: held ? s.unread : { ...s.unread, [name]: 0 },
    holdRead,
    ui: {
      ...s.ui, activeSpace: name, history, future, editing: undefined, browseOpen: false, switcher: false, composeOpen: false, feed: undefined, todosSpace: undefined, spaceMenu: false,
      unreadAtOpen: held ? s.ui.unreadAtOpen : s.unread[name] ?? 0,
      search: { ...s.ui.search, open: false },
      ...(opts.keepPanel ? {} : { thread: undefined, panel: s.ui.panel === "members" ? "members" : "none" }),
    },
  });
  localStorage.setItem(LAST_SPACE_KEY, name);
  void ensureMembers(name);
  void ensureMessages(name);
}

export function goBack(): void {
  const s = S.getState();
  const prev = s.ui.history[s.ui.history.length - 1];
  if (prev) openSpace(prev, { nav: "back" });
}
export function goForward(): void {
  const s = S.getState();
  const next = s.ui.future[0];
  if (next) openSpace(next, { nav: "forward" });
}

export function stepSpace(delta: 1 | -1, unreadOnly = false): void {
  const s = S.getState();
  const order = orderOf(s);
  if (!order.length) return;
  const i = order.findIndex((x) => x.name === s.ui.activeSpace);
  for (let k = 1; k <= order.length; k++) {
    const sp = order[(i + delta * k + order.length * k) % order.length]!;
    if (!unreadOnly || (s.unread[sp.name] ?? 0) > 0) { openSpace(sp.name); return; }
  }
}

export async function ensureMembers(space: string): Promise<void> {
  if (S.getState().members[space]) return;
  try {
    const list = await bridge.listMembers(space);
    S.setState((s) => ({ members: { ...s.members, [space]: list } }));
  } catch (e) {
    console.warn("listMembers", space, e);
  }
}

export const searchPeople = (query: string): Promise<User[]> => bridge.searchPeople(query);

/** Starts (or reopens) a direct message with `user`. Throws so the dialog can show why. */
export async function openDm(user: User): Promise<void> {
  adopt(await bridge.setupDm(user.name), [user]);
}
/** One person opens a DM, two or more open (or create) a group chat. */
export async function startConversation(users: User[]): Promise<void> {
  if (users.length === 0) return;
  if (users.length === 1) return openDm(users[0]!);
  adopt(await bridge.setupGroup(users.map((u) => u.name)), users);
}
/** Creates a named space with `members` (the server adds me) and opens it. Throws on ALREADY_EXISTS and friends. */
export async function createSpace(displayName: string, members: User[]): Promise<void> {
  adopt(await bridge.createSpace(displayName, members.map((u) => u.name)), members);
}
/** A space the server just handed back: upsert it, seed members so the label and avatar render, subscribe, open, then refresh the members from the server. */
function adopt(space: Space, users: User[]): void {
  const s = S.getState();
  const known = s.spaces.some((x) => x.name === space.name);
  const seed = [...users, ...(s.auth.user ? [s.auth.user] : [])].map((u) => ({
    name: `${space.name}/members/${u.name.replace("users/", "")}`, state: "JOINED" as const, role: "ROLE_MEMBER" as const, member: u,
  }));
  S.setState({
    spaces: known ? s.spaces.map((x) => (x.name === space.name ? { ...x, ...space } : x)) : [...s.spaces, space],
    members: s.members[space.name] ? s.members : { ...s.members, [space.name]: seed },
  });
  if (!known) subscribeRelay();
  openSpace(space.name);
  void refreshMembers(space.name);
}

/** Server member list over the seeded one; names and emails the server leaves blank stay as seeded. */
export async function refreshMembers(space: string): Promise<void> {
  try {
    const list = await bridge.listMembers(space);
    S.setState((s) => {
      const seeded = nameMapOf(s.members[space]);
      const merged = list.map((m) => (m.member ? { ...m, member: { ...(seeded[m.member.name] ?? {}), ...stripEmpty(m.member) } } : m));
      return { members: { ...s.members, [space]: merged } };
    });
  } catch (e) {
    console.warn("refreshMembers", space, e);
  }
}
const nameMapOf = (list: State["members"][string] | undefined): Record<string, User> => {
  const out: Record<string, User> = {};
  for (const m of list ?? []) if (m.member) out[m.member.name] = m.member;
  return out;
};
const stripEmpty = (u: User): User => Object.fromEntries(Object.entries(u).filter(([, v]) => v !== undefined && v !== "")) as User;

// Drive attachments arrive as {name, source}; one GET per attachment fills the card in and is remembered across reloads.
const resolvedAtt = new Map<string, Attachment>();
const resolving = new Set<string>();
export async function resolveAttachment(message: string, att: Attachment): Promise<void> {
  const key = att.name;
  if (!key || resolving.has(key)) return;
  const apply = (full: Attachment) => patchMessage(message, (m) => ({ ...m, attachment: (m.attachment ?? []).map((a) => (a.name === key ? { ...a, ...full } : a)) }));
  const hit = resolvedAtt.get(key);
  if (hit) { apply(hit); return; }
  // Google refuses attachments.get for user auth; the share message's own link names the Drive file.
  const local = driveFromText(att, findMessage(message)?.formattedText);
  if (local) { resolvedAtt.set(key, local); apply(local); return; }
  resolving.add(key);
  try {
    const full = await bridge.getAttachment(key);
    resolvedAtt.set(key, full);
    apply(full);
  } catch (e) {
    console.warn("attachment", key, e);
    // note: a failed fetch leaves the stub; the card then shows a generic "File" label with nothing to open.
    resolvedAtt.set(key, { ...att, contentName: att.contentName ?? "File" });
    apply(resolvedAtt.get(key)!);
  } finally {
    resolving.delete(key);
  }
}

function absorbMine(items: Message[]): void {
  const add: Record<string, string[]> = {};
  for (const m of items) if (m._myReactions) add[m.name] = m._myReactions;
  if (Object.keys(add).length) S.setState((s) => ({ myReactions: { ...s.myReactions, ...add } }));
}

export async function ensureMessages(space: string): Promise<void> {
  const page = S.getState().messages[space];
  if (page?.loaded || page?.loading) return;
  S.setState((s) => ({ messages: { ...s.messages, [space]: { ...emptyPage(), loading: true } } }));
  try {
    const r = await bridge.listMessages(space, { pageSize: 50 });
    absorbMine(r.items);
    S.setState((s) => {
      const existing = s.messages[space]?.items ?? [];
      let items = existing;
      for (const m of r.items) items = upsert(items, m);
      return { messages: { ...s.messages, [space]: { items, nextPageToken: r.nextPageToken, loaded: true, loading: false } } };
    });
  } catch (e) {
    console.warn("listMessages", space, e);
    S.setState((s) => ({ messages: { ...s.messages, [space]: { ...(s.messages[space] ?? emptyPage()), loaded: true, loading: false } } }));
  }
}

export async function loadOlder(space: string): Promise<boolean> {
  const page = S.getState().messages[space];
  if (!page?.nextPageToken || page.loading) return false;
  S.setState((s) => ({ messages: { ...s.messages, [space]: { ...s.messages[space]!, loading: true } } }));
  try {
    const r = await bridge.listMessages(space, { pageSize: 50, pageToken: page.nextPageToken });
    absorbMine(r.items);
    S.setState((s) => {
      let items = s.messages[space]!.items;
      for (const m of r.items) items = upsert(items, m);
      return { messages: { ...s.messages, [space]: { items, nextPageToken: r.nextPageToken, loaded: true, loading: false } } };
    });
    return true;
  } catch (e) {
    console.warn("loadOlder", e);
    S.setState((s) => ({ messages: { ...s.messages, [space]: { ...s.messages[space]!, loading: false } } }));
    return false;
  }
}

export function openThread(thread: string): void {
  const s = S.getState();
  S.setState({ ui: { ...s.ui, thread, panel: "thread", editing: undefined } });
  const space = thread.split("/").slice(0, 2).join("/");
  const page = s.threads[thread];
  if (page?.loaded || page?.loading) return;
  S.setState((st) => ({ threads: { ...st.threads, [thread]: { ...emptyPage(), loading: true } } }));
  bridge.listMessages(space, { thread, pageSize: 100 })
    .then((r) => {
      absorbMine(r.items);
      S.setState((st) => {
        let items = st.threads[thread]?.items ?? [];
        for (const m of r.items) items = upsert(items, m);
        return { threads: { ...st.threads, [thread]: { items, loaded: true, loading: false } } };
      });
    })
    .catch((e) => {
      console.warn("thread", e);
      S.setState((st) => ({ threads: { ...st.threads, [thread]: { ...(st.threads[thread] ?? emptyPage()), loaded: true, loading: false } } }));
    });
}

export function setPanel(panel: State["ui"]["panel"]): void {
  S.setState((s) => ({ ui: { ...s.ui, panel, ...(panel === "none" ? { thread: undefined } : {}) } }));
}
export function toggleThreadPanel(): void {
  const s = S.getState();
  if (s.ui.panel === "thread") setPanel("none");
  else if (s.ui.thread) setPanel("thread");
}

function putMessage(msg: Message): void {
  const space = msg.name.split("/").slice(0, 2).join("/");
  S.setState((s) => {
    const page = s.messages[space] ?? emptyPage();
    const threads = { ...s.threads };
    const t = msg.thread?.name;
    if (t && threads[t]) threads[t] = { ...threads[t]!, items: upsert(threads[t]!.items, msg) };
    return { messages: { ...s.messages, [space]: { ...page, items: upsert(page.items, msg) } }, threads };
  });
}
function dropMessage(name: string): void {
  const space = name.split("/").slice(0, 2).join("/");
  S.setState((s) => {
    const page = s.messages[space];
    const threads: State["threads"] = {};
    for (const [k, v] of Object.entries(s.threads)) threads[k] = { ...v, items: remove(v.items, name) };
    return { messages: page ? { ...s.messages, [space]: { ...page, items: remove(page.items, name) } } : s.messages, threads };
  });
}
/** Google keeps a deleted message in the list as "Message deleted": content gone, sender and time kept. */
function tombstone(name: string, meta: Pick<Message, "deleteTime" | "deletionMetadata"> = {}): void {
  patchMessage(name, (m) => ({ name: m.name, sender: m.sender, createTime: m.createTime, thread: m.thread, threadReply: m.threadReply, deleteTime: meta.deleteTime ?? new Date().toISOString(), deletionMetadata: meta.deletionMetadata }));
}
function patchMessage(name: string, fn: (m: Message) => Message): void {
  const space = name.split("/").slice(0, 2).join("/");
  S.setState((s) => {
    const page = s.messages[space];
    const threads: State["threads"] = {};
    for (const [k, v] of Object.entries(s.threads)) threads[k] = { ...v, items: patch(v.items, name, fn) };
    return { messages: page ? { ...s.messages, [space]: { ...page, items: patch(page.items, name, fn) } } : s.messages, threads };
  });
}
function findMessage(name: string): Message | undefined {
  const s = S.getState();
  const space = name.split("/").slice(0, 2).join("/");
  return s.messages[space]?.items.find((m) => m.name === name) ?? Object.values(s.threads).flatMap((t) => t.items).find((m) => m.name === name);
}

export async function send(space: string, text: string, thread?: string, attachments?: Attachment[]): Promise<void> {
  const clientId = uuid();
  typing(space, thread, false);
  let optimistic: Message;
  try {
    optimistic = await bridge.sendMessage(space, text, { thread, clientId, attachments });
  } catch (e) {
    optimistic = { name: `${space}/messages/client-${clientId}`, text, createTime: new Date().toISOString(), sender: S.getState().auth.user, _pending: "failed", _error: String(e) };
  }
  const m: Message = {
    ...optimistic,
    sender: optimistic.sender ?? S.getState().auth.user,
    createTime: optimistic.createTime ?? new Date().toISOString(),
    clientAssignedMessageId: optimistic.clientAssignedMessageId ?? `client-${clientId}`,
    _pending: optimistic._pending ?? "sending",
    ...(thread ? { thread: { name: thread }, threadReply: true } : {}),
    ...(attachments?.length && !optimistic.attachment ? { attachment: attachments } : {}),
  };
  putMessage(m);
}

export async function retry(m: Message): Promise<void> {
  dropMessage(m.name);
  const space = m.name.split("/").slice(0, 2).join("/");
  await send(space, bodyOf(m), m.threadReply ? m.thread?.name : undefined, m.attachment);
}

export async function uploadAndSend(space: string, file: File, thread?: string): Promise<void> {
  const att = await bridge.uploadAttachment(space, file);
  await send(space, "", thread, [att]);
}
export async function uploadPathAndSend(space: string, path: string, thread?: string): Promise<void> {
  if (!bridge.uploadAttachmentPath) return;
  const att = await bridge.uploadAttachmentPath(space, path);
  await send(space, "", thread, [att]);
}

export async function edit(name: string, text: string): Promise<void> {
  S.setState((s) => ({ ui: { ...s.ui, editing: undefined } }));
  patchMessage(name, (m) => ({ ...m, text }));
  try {
    const r = await bridge.editMessage(name, text);
    putMessage(r);
  } catch (e) {
    console.warn("edit", e);
  }
}
/** Deletes for everyone; `force` also removes the thread's replies, which Google requires for a parent with replies. */
export async function del(name: string, force = false): Promise<void> {
  ui({ deleting: undefined });
  const space = spaceOf(name);
  const replies = force ? (S.getState().messages[space]?.items ?? []).filter((m) => m.threadReply && m.thread?.name && findMessage(name)?.thread?.name === m.thread.name).map((m) => m.name) : [];
  const meta = { deletionMetadata: { deletionType: findMessage(name)?.sender?.name === me() ? "CREATOR" : "SPACE_OWNER" } };
  tombstone(name, meta);
  for (const r of replies) tombstone(r, meta);
  try { await bridge.deleteMessage(name, force); toast(replies.length ? `Message and ${replies.length} ${replies.length === 1 ? "reply" : "replies"} deleted` : "Message deleted"); }
  catch (e) { console.warn("delete", e); toast(`Could not delete: ${String(e)}`); }
}

function bump(sums: EmojiReactionSummary[] | undefined, unicode: string, delta: number): EmojiReactionSummary[] {
  const list = [...(sums ?? [])];
  const i = list.findIndex((x) => x.emoji?.unicode === unicode);
  if (i >= 0) {
    const c = (list[i]!.reactionCount ?? 0) + delta;
    if (c <= 0) list.splice(i, 1);
    else list[i] = { ...list[i]!, reactionCount: c };
  } else if (delta > 0) list.push({ emoji: { unicode }, reactionCount: delta });
  return list;
}
function setMine(name: string, unicode: string, on: boolean): void {
  S.setState((s) => {
    const cur = s.myReactions[name] ?? [];
    const next = on ? (cur.includes(unicode) ? cur : [...cur, unicode]) : cur.filter((u) => u !== unicode);
    return { myReactions: { ...s.myReactions, [name]: next } };
  });
}

export async function toggleReaction(name: string, unicode: string): Promise<void> {
  const had = (S.getState().myReactions[name] ?? []).includes(unicode);
  setMine(name, unicode, !had);
  patchMessage(name, (m) => ({ ...m, emojiReactionSummaries: bump(m.emojiReactionSummaries, unicode, had ? -1 : 1) }));
  try {
    await bridge.toggleReaction(name, unicode);
  } catch (e) {
    console.warn("reaction", e);
    setMine(name, unicode, had);
    patchMessage(name, (m) => ({ ...m, emojiReactionSummaries: bump(m.emojiReactionSummaries, unicode, had ? 1 : -1) }));
  }
}

const lastMarked: Record<string, string> = {};
export function markRead(space: string, message: string, thread?: string): void {
  const key = draftKey(space, thread);
  if (lastMarked[key] === message || clientIdOf(message) || S.getState().holdRead[space]) return;
  lastMarked[key] = message;
  bridge.markRead(space, message, thread).catch((e) => console.warn("markRead", e));
  if (S.getState().prefs.readReceipts) bridge.relaySend({ type: "read", space, thread, message });
}

export function typing(space: string, thread: string | undefined, active: boolean): void {
  bridge.relaySend({ type: "typing", space, thread, active });
}

/**
 * Presence goes two ways: the relay tells other Confab users at once, and Google Chat is asked to mark the same
 * state (users.availability), so Gmail and Chat show it too. `dndSeconds` bounds Do not disturb; `statusSeconds`
 * bounds the custom status (Google requires an emoji with it; 💬 stands in).
 */
let activeRefresh = 0;
export function setPresence(state: PresenceState, custom?: CustomStatus, opts: { dndSeconds?: number; statusSeconds?: number; pushStatus?: boolean } = {}): void {
  const before = S.getState().me;
  S.setState({ me: { state, custom } });
  bridge.relaySend({ type: "presence", state, custom_status: custom });
  const stateChanged = before.state !== state;
  const statusChanged = opts.pushStatus ?? (before.custom?.text !== custom?.text || before.custom?.emoji !== custom?.emoji);
  if (stateChanged && state !== "offline") {
    bridge.availabilityMark(state === "dnd" ? "dnd" : state === "away" ? "away" : "active", state === "dnd" ? (opts.dndSeconds ?? 3600) : undefined)
      .catch((e) => { console.warn("availability", e); toast(`Google did not take the status: ${String(e).slice(0, 90)}`); });
  }
  if (statusChanged) {
    bridge.availabilityStatus(custom?.text || undefined, custom?.emoji || undefined, opts.statusSeconds)
      .catch((e) => { console.warn("custom status", e); toast(`Google did not take the status note: ${String(e).slice(0, 90)}`); });
  }
  // Google's ACTIVE expires after half an hour; while Confab is the user's chosen state, keep it fresh.
  window.clearInterval(activeRefresh);
  if (state === "active") activeRefresh = window.setInterval(() => { if (document.hasFocus()) bridge.availabilityMark("active", 1800).catch(() => {}); }, 25 * 60_000);
}
/** On sign-in: Google's view of my state and status seeds the rail, so Confab and Gmail agree from the first second. */
export async function syncPresenceFromGoogle(): Promise<void> {
  try {
    const a = await bridge.availabilityGet();
    const state: PresenceState = a.state === "AWAY" ? "away" : a.state === "DO_NOT_DISTURB" ? "dnd" : "active";
    const custom = a.customStatus?.text ? { text: a.customStatus.text, emoji: a.customStatus.emoji?.unicode, expires_at: a.customStatus.expireTime } : undefined;
    S.setState({ me: { state, custom } });
    bridge.relaySend({ type: "presence", state, custom_status: custom });
  } catch (e) {
    console.warn("availability get", e);
  }
}

const draftTimers: Record<string, number> = {};
export function saveDraft(space: string, thread: string | undefined, text: string): void {
  const key = draftKey(space, thread);
  S.setState((s) => ({ drafts: { ...s.drafts, [key]: text } }));
  window.clearTimeout(draftTimers[key]);
  draftTimers[key] = window.setTimeout(() => bridge.relaySend({ type: "draft", space, thread, text }), 800);
}

export async function runSearch(query: string): Promise<void> {
  const q = query.trim();
  S.setState((s) => ({ ui: { ...s.ui, search: { open: !!q, query, results: q ? s.ui.search.results : [], loading: !!q } } }));
  if (!q) return;
  try {
    const results = await bridge.search(q);
    S.setState((s) => (s.ui.search.query === query ? { ui: { ...s.ui, search: { open: true, query, results, loading: false } } } : {}));
  } catch (e) {
    console.warn("search", e);
    S.setState((s) => ({ ui: { ...s.ui, search: { ...s.ui.search, loading: false } } }));
  }
}
export function closeSearch(): void {
  S.setState((s) => ({ ui: { ...s.ui, search: { ...s.ui.search, open: false } } }));
}

export function toggleStar(space: string): void {
  S.setState((s) => ({ starred: s.starred.includes(space) ? s.starred.filter((x) => x !== space) : [...s.starred, space] }));
}
export function toggleMute(space: string): void {
  const on = !S.getState().muted.includes(space);
  S.setState((s) => ({ muted: on ? [...s.muted, space] : s.muted.filter((x) => x !== space) }));
  toast(on ? "Muted. No badge, no sound." : "Unmuted");
}
export function setPrefs(p: Partial<Prefs>): void {
  S.setState((s) => ({ prefs: { ...s.prefs, ...p } }));
}
/** Page zoom on desktop (the webview scales everything, so coordinates stay consistent); browsers have their own. */
export function zoom(delta: 0.1 | -0.1 | 0): void {
  if (!bridge.setZoom) { toast("Use your browser's zoom here"); return; }
  const cur = S.getState().prefs.zoom;
  const next = delta === 0 ? 1 : Math.round(Math.min(1.6, Math.max(0.7, cur + delta)) * 100) / 100;
  if (next === cur) return;
  setPrefs({ zoom: next });
  void bridge.setZoom(next);
  toast(`Zoom ${Math.round(next * 100)}%`, 1200);
}

const spaceOf = (message: string) => message.split("/").slice(0, 2).join("/");
export function isSaved(name: string): boolean {
  return S.getState().saved.some((x) => x.name === name);
}
export function toggleSaved(m: Message): void {
  const was = isSaved(m.name);
  S.setState((s) => ({
    saved: was ? s.saved.filter((x) => x.name !== m.name) : [{ name: m.name, space: spaceOf(m.name), at: new Date().toISOString(), text: bodyOf(m), sender: m.sender, createTime: m.createTime }, ...s.saved],
  }));
  toast(was ? "Removed from saved" : "Saved for later");
}
/** Marks the space unread from `message` on: the badge counts it and the next open shows the "New" line there. */
export function markUnread(message: string): void {
  const space = spaceOf(message);
  const s = S.getState();
  const top = (s.messages[space]?.items ?? []).filter((m) => !m.threadReply);
  const i = top.findIndex((m) => m.name === message);
  const n = i >= 0 ? top.length - i : 1;
  S.setState({ unread: { ...s.unread, [space]: n }, holdRead: { ...s.holdRead, [space]: true } });
  delete lastMarked[space];
  toast(`Marked unread from here. ${n} ${n === 1 ? "message" : "messages"}.`);
}
export function markSpaceRead(space: string): void {
  const s = S.getState();
  const holdRead = { ...s.holdRead };
  delete holdRead[space];
  S.setState({ unread: { ...s.unread, [space]: 0 }, holdRead });
  const last = [...(s.messages[space]?.items ?? [])].reverse().find((m) => !m.threadReply && !m._pending);
  if (last) markRead(space, last.name);
}
export function markAllRead(): void {
  for (const [space, n] of Object.entries(S.getState().unread)) if (n > 0) markSpaceRead(space);
  toast("All caught up");
}
export function rememberEmoji(u: string): void {
  S.setState((s) => ({ recentEmoji: [u, ...s.recentEmoji.filter((x) => x !== u)].slice(0, 12) }));
}
export function openFeed(feed: Feed): void {
  const s = S.getState();
  if (s.ui.feed === feed) { closeFeed(); return; }
  if (feed === "unreads") for (const [space, n] of Object.entries(s.unread)) if (n > 0) void ensureMessages(space);
  if (feed === "todos") void loadTasks();
  if (feed === "calendar") void openCalendar();
  if (feed === "notes") void loadNotes();
  if (feed === "drive") void openDrive();
  // The directory view lists everyone from every space; DMs loaded at boot, the rest load now (cached after).
  if (feed === "people") for (const sp of s.spaces) if (sp.spaceType !== "DIRECT_MESSAGE") void ensureMembers(sp.name);
  ui({ feed, search: { ...s.ui.search, open: false }, switcher: false, composeOpen: false });
}
export function closeFeed(): void {
  ui({ feed: undefined, todosSpace: undefined });
  stopCalendarPolling();
}
/** Opens the message's space and scrolls to it; the list flashes the row when it lands. */
export function jumpTo(message: string): void {
  const space = spaceOf(message);
  openSpace(space, { keepPanel: true });
  ui({ jumpTo: { message, at: Date.now() } });
}
// ---- Google Tasks (default list) ----
export async function loadTasks(): Promise<void> {
  try {
    const items = await bridge.listTasks();
    S.setState({ tasks: { items: sortTasks(items), loaded: true } });
  } catch (e) {
    console.warn("tasks", e);
    S.setState((s) => ({ tasks: { ...s.tasks, loaded: true, error: String(e) } }));
  }
}
/** Open first, soonest due first, then newest; done at the bottom. */
export function sortTasks(items: Task[]): Task[] {
  const key = (t: Task) => (t.status === "completed" ? "2" : "0") + (t.due ? "0" + t.due : "1") + (t.updated ?? "");
  return [...items].sort((a, b) => (key(a) < key(b) ? -1 : 1));
}
export async function addTask(title: string, notes?: string, due?: string): Promise<void> {
  const t = title.trim();
  if (!t) return;
  try {
    const task = await bridge.addTask(t, notes, due);
    S.setState((s) => ({ tasks: { ...s.tasks, items: sortTasks([task, ...s.tasks.items.filter((x) => x.id !== task.id)]) } }));
    toast("Added to Google Tasks");
  } catch (e) {
    toast(`Could not add the task: ${String(e)}`);
  }
}
export async function toggleTask(task: Task): Promise<void> {
  const done = task.status !== "completed";
  const flip = (t: Task): Task => ({ ...t, status: done ? "completed" : "needsAction" });
  S.setState((s) => ({ tasks: { ...s.tasks, items: s.tasks.items.map((x) => (x.id === task.id ? flip(x) : x)) } }));
  try {
    const fresh = await bridge.setTaskDone(task.id, done);
    S.setState((s) => ({ tasks: { ...s.tasks, items: sortTasks(s.tasks.items.map((x) => (x.id === task.id ? { ...x, ...fresh } : x))) } }));
  } catch (e) {
    toast(`Could not update the task: ${String(e)}`);
    S.setState((s) => ({ tasks: { ...s.tasks, items: s.tasks.items.map((x) => (x.id === task.id ? task : x)) } }));
  }
}
export async function removeTask(id: string): Promise<void> {
  const before = S.getState().tasks.items;
  S.setState((s) => ({ tasks: { ...s.tasks, items: s.tasks.items.filter((x) => x.id !== id) } }));
  try { await bridge.deleteTask(id); } catch (e) { toast(`Could not delete the task: ${String(e)}`); S.setState((s) => ({ tasks: { ...s.tasks, items: before } })); }
}
/** "Add to Tasks" on a message: the first line as the title, the message link as the notes. */
export function taskFromMessage(m: Message, names: Record<string, User>): Promise<void> {
  const text = plainText(bodyOf(m), (u) => names[u]?.displayName ?? u) || "Message with a file";
  const title = text.length > 100 ? `${text.slice(0, 97)}…` : text;
  const [, sid, , mid] = m.name.split("/");
  const who = m.sender?.displayName ?? "Someone";
  return addTask(title, `From ${who} in Confab\nhttps://mail.google.com/chat/u/0/#chat/space/${sid}/${mid}`);
}

// ---- space options: pins (local), members, Meet, tasks from here ----
export function togglePin(message: string): void {
  const space = spaceOf(message);
  const was = (S.getState().pinned[space] ?? []).includes(message);
  S.setState((s) => ({ pinned: { ...s.pinned, [space]: was ? (s.pinned[space] ?? []).filter((x) => x !== message) : [message, ...(s.pinned[space] ?? [])] } }));
  toast(was ? "Unpinned" : "Pinned in this space (on this device)");
}
export async function addMembers(space: string, users: User[]): Promise<void> {
  const list = await bridge.addMembers(space, users.map((u) => u.name));
  S.setState((s) => {
    const cur = s.members[space] ?? [];
    const names = new Set(cur.map((m) => m.name));
    return { members: { ...s.members, [space]: [...cur, ...list.filter((m) => !names.has(m.name))] } };
  });
  toast(`Added ${users.length === 1 ? users[0]!.displayName ?? "1 person" : `${users.length} people`}`);
}
export async function leaveSpace(space: string): Promise<void> {
  await bridge.leaveSpace(space);
  const s = S.getState();
  const rest = s.spaces.filter((x) => x.name !== space);
  S.setState({ spaces: rest, ui: { ...s.ui, spaceMenu: false, panel: "none" } });
  if (s.ui.activeSpace === space) { const next = orderOf({ ...s, spaces: rest })[0]; if (next) openSpace(next.name); else ui({ activeSpace: undefined }); }
  toast("You left the space");
}
/**
 * Google Chat's video-call button: a new Meet whose link is posted into the space for everyone, then opened here.
 * Without a space, or when the Meet API is not available, it opens a fresh meeting in the browser instead.
 */
let meetStarting = false;
export async function startMeet(space?: string): Promise<void> {
  if (!space) { void openExternal("https://meet.google.com/new"); return; }
  if (meetStarting) return;
  meetStarting = true;
  toast("Starting a video call…", 2500);
  try {
    const meet = await bridge.createMeet();
    void openExternal(meet.meetingUri);
    await send(space, `Join the video call: ${meet.meetingUri}`);
  } catch (e) {
    console.warn("createMeet", e);
    toast("Meet API did not answer; opened a new meeting instead. Paste its link here to share it.", 4500);
    void openExternal("https://meet.google.com/new");
  } finally {
    meetStarting = false;
  }
}
/** Todos narrowed to tasks created from this space (their notes carry the message link). */
export function openSpaceTasks(space: string): void {
  ui({ todosSpace: space });
  openFeed("todos");
}

// ---- accounts: one profile per Google account, switched from the rail ----
export async function loadAccounts(): Promise<void> {
  try { S.setState({ accounts: await bridge.accountsList() }); } catch (e) { console.warn("accounts", e); }
}
/** Everything the active account owned is dropped, then the workspace boots again for the new one. */
function resetForAccount(): void {
  leaveHuddle();
  stopCalendarPolling();
  const s = S.getState();
  S.setState({
    auth: { status: "loading" }, spaces: [], members: {}, messages: {}, threads: {}, unread: {}, typing: {}, presence: {}, receipts: {}, myReactions: {},
    relay: "closed", holdRead: {}, drafts: {}, tasks: { items: [], loaded: false }, previews: {}, huddles: {},
    calendar: { ...s.calendar, calendars: [], events: {}, syncTokens: {}, loaded: false, loading: false, error: undefined, selected: undefined, editing: undefined },
    notes: { ...s.notes, items: [], loaded: false, error: undefined, editing: undefined },
    ui: { ...s.ui, activeSpace: undefined, thread: undefined, panel: "none", prefsOpen: false, switcher: false, composeOpen: false, titleMenu: false, statusMenu: false, spaceMenu: false, history: [], future: [], feed: undefined, jumpTo: undefined, viewer: undefined, deleting: undefined, search: { ...s.ui.search, open: false } },
  });
}
async function rebootWorkspace(): Promise<void> {
  try {
    const st = await bridge.authStatus();
    if (st.signedIn) {
      S.setState({ auth: { status: "in", user: st.user } });
      await loadWorkspace();
    } else S.setState({ auth: { status: "out" } });
  } catch (e) {
    S.setState({ auth: { status: "out", error: String(e) } });
  }
}
export async function switchAccount(id: string): Promise<void> {
  const s = S.getState();
  if (s.accounts.find((a) => a.active)?.id === id) return;
  try {
    const a = await bridge.accountSwitch(id);
    resetForAccount();
    S.setState((st) => ({ accounts: st.accounts.map((x) => ({ ...x, active: x.id === id, unread: x.id === id ? 0 : x.unread })) }));
    await rebootWorkspace();
    toast(`Now in ${a.displayName ?? a.email ?? id}`);
  } catch (e) {
    toast(`Could not switch: ${String(e)}`);
  }
}
/** A fresh profile, signed out: the sign-in screen appears with a way back to the previous account. */
export async function addAccount(): Promise<void> {
  const prev = S.getState().accounts.find((a) => a.active)?.id;
  try {
    const a = await bridge.accountAdd();
    resetForAccount();
    S.setState((st) => ({ accounts: [...st.accounts.map((x) => ({ ...x, active: false })), { ...a, active: true }], auth: { status: "out" }, ui: { ...st.ui, addingFrom: prev } }));
  } catch (e) {
    toast(String(e));
  }
}
export async function removeAccount(id: string): Promise<void> {
  try {
    await bridge.accountRemove(id);
    S.setState((s) => ({ accounts: s.accounts.filter((a) => a.id !== id) }));
    toast("Account removed from this Confab");
  } catch (e) {
    toast(`Could not remove: ${String(e)}`);
  }
}
/** Cancel adding: back to the account we came from, and the empty profile goes. */
export async function cancelAddAccount(): Promise<void> {
  const s = S.getState();
  const empty = s.accounts.find((a) => a.active && !a.signedIn)?.id;
  const back = s.ui.addingFrom ?? s.accounts.find((a) => a.signedIn && !a.active)?.id;
  if (!back) return;
  ui({ addingFrom: undefined });
  await switchAccount(back);
  if (empty) void removeAccount(empty);
}

// ---- Google Calendar: a window of events per calendar, then sync tokens every half minute while the view is open ----
const CAL_BACK_DAYS = 35;
const CAL_AHEAD_DAYS = 70;
let calPoll = 0;
let calReminderTimer = 0;
const remindedFor = new Set<string>();
const calKey = (e: CalEvent) => `${e.calendarId ?? "primary"}:${e.id}`;

export async function openCalendar(): Promise<void> {
  const s = S.getState().calendar;
  if (!s.loaded && !s.loading) await loadCalendar();
  else void pollCalendar();
  window.clearInterval(calPoll);
  calPoll = window.setInterval(() => void pollCalendar(), 30_000);
}
export function stopCalendarPolling(): void {
  window.clearInterval(calPoll);
  calPoll = 0;
}
/** Full load: the calendar list, then every visible calendar's events in a window around today, keeping each sync token. */
export async function loadCalendar(): Promise<void> {
  S.setState((s) => ({ calendar: { ...s.calendar, loading: true, error: undefined } }));
  try {
    const calendars = await bridge.calendarList();
    const min = addDays(new Date(), -CAL_BACK_DAYS).toISOString();
    const max = addDays(new Date(), CAL_AHEAD_DAYS).toISOString();
    const events: Record<string, CalEvent> = {};
    const syncTokens: Record<string, string> = {};
    await Promise.all(calendars.map(async (c) => {
      try {
        const page = await bridge.calendarEvents(c.id, { timeMin: min, timeMax: max });
        for (const e of page.items) if (e.status !== "cancelled") events[calKey({ ...e, calendarId: c.id })] = { ...e, calendarId: c.id };
        if (page.nextSyncToken) syncTokens[c.id] = page.nextSyncToken;
      } catch (e) { console.warn("calendar", c.id, e); }
    }));
    S.setState((s) => ({ calendar: { ...s.calendar, calendars, events, syncTokens, loaded: true, loading: false, polledAt: Date.now() } }));
    scheduleReminders();
  } catch (e) {
    S.setState((s) => ({ calendar: { ...s.calendar, loading: false, loaded: true, error: String(e) } }));
  }
}
/** Incremental: each calendar's sync token brings only what changed, cancelled events included; an expired token reloads. */
export async function pollCalendar(): Promise<void> {
  const s = S.getState().calendar;
  if (!s.loaded || s.loading) return;
  let expired = false;
  const changes: Record<string, CalEvent | null> = {};
  const tokens = { ...s.syncTokens };
  await Promise.all(s.calendars.map(async (c) => {
    const token = s.syncTokens[c.id];
    if (!token) return;
    try {
      const page = await bridge.calendarEvents(c.id, { syncToken: token });
      for (const e of page.items) changes[calKey({ ...e, calendarId: c.id })] = e.status === "cancelled" ? null : { ...e, calendarId: c.id };
      if (page.nextSyncToken) tokens[c.id] = page.nextSyncToken;
    } catch (e) {
      if (String(e).includes("sync_expired")) expired = true;
      else console.warn("calendar poll", c.id, e);
    }
  }));
  if (expired) { await loadCalendar(); return; }
  if (Object.keys(changes).length) {
    S.setState((st) => {
      const events = { ...st.calendar.events };
      for (const [k, v] of Object.entries(changes)) { if (v) events[k] = v; else delete events[k]; }
      return { calendar: { ...st.calendar, events, syncTokens: tokens, polledAt: Date.now() } };
    });
    scheduleReminders();
  } else S.setState((st) => ({ calendar: { ...st.calendar, syncTokens: tokens, polledAt: Date.now() } }));
}
export function setCalView(view: CalView): void {
  S.setState((s) => ({ calendar: { ...s.calendar, view } }));
}
export function setCalDay(day: string): void {
  S.setState((s) => ({ calendar: { ...s.calendar, day } }));
}
export function calStep(dir: 1 | -1): void {
  const s = S.getState().calendar;
  const [y, m, d] = s.day.split("-").map(Number);
  const cur = new Date(y!, m! - 1, d!);
  const next = s.view === "month" ? new Date(y!, m! - 1 + dir, 1) : s.view === "week" ? addDays(cur, 7 * dir) : s.view === "agenda" ? addDays(cur, 14 * dir) : addDays(cur, dir);
  setCalDay(todayKey(next));
}
export function selectEvent(key?: string): void {
  S.setState((s) => ({ calendar: { ...s.calendar, selected: key } }));
}
/** Opens the editor: a new event at `start` (or now, rounded up), or an existing one by key. */
export function editEvent(init?: Partial<CalEvent> | null): void {
  S.setState((s) => ({ calendar: { ...s.calendar, editing: init ?? null, selected: undefined } }));
}
export function closeEventEditor(): void {
  S.setState((s) => ({ calendar: { ...s.calendar, editing: undefined } }));
}
export function toggleCalendar(id: string): void {
  S.setState((s) => ({ calendar: { ...s.calendar, hidden: s.calendar.hidden.includes(id) ? s.calendar.hidden.filter((x) => x !== id) : [...s.calendar.hidden, id] } }));
}
export async function saveEvent(calendarId: string, input: EventInput, existing?: CalEvent): Promise<void> {
  const e = existing ? await bridge.calendarUpdateEvent(existing.calendarId ?? calendarId, existing.id, input) : await bridge.calendarCreateEvent(calendarId, input);
  S.setState((s) => ({ calendar: { ...s.calendar, events: { ...s.calendar.events, [calKey(e)]: e }, editing: undefined, selected: calKey(e) } }));
  toast(existing ? "Event updated" : input.attendees.length ? "Invitations sent" : "Event created");
  scheduleReminders();
}
export async function respondEvent(e: CalEvent, response: "accepted" | "declined" | "tentative"): Promise<void> {
  const before = e;
  const optimistic = { ...e, attendees: (e.attendees ?? []).map((a) => (a.self ? { ...a, responseStatus: response } : a)) };
  S.setState((s) => ({ calendar: { ...s.calendar, events: { ...s.calendar.events, [calKey(e)]: optimistic } } }));
  try {
    const fresh = await bridge.calendarRespond(e.calendarId ?? "primary", e.id, response);
    S.setState((s) => ({ calendar: { ...s.calendar, events: { ...s.calendar.events, [calKey(e)]: { ...fresh, calendarId: e.calendarId } } } }));
  } catch (err) {
    S.setState((s) => ({ calendar: { ...s.calendar, events: { ...s.calendar.events, [calKey(e)]: before } } }));
    toast(`Could not respond: ${String(err)}`);
  }
}
export async function deleteEvent(e: CalEvent): Promise<void> {
  S.setState((s) => { const events = { ...s.calendar.events }; delete events[calKey(e)]; return { calendar: { ...s.calendar, events, selected: undefined, editing: undefined } }; });
  try { await bridge.calendarDeleteEvent(e.calendarId ?? "primary", e.id); toast("Event deleted"); }
  catch (err) { toast(`Could not delete: ${String(err)}`); S.setState((s) => ({ calendar: { ...s.calendar, events: { ...s.calendar.events, [calKey(e)]: e } } })); }
}
/** A notification ten minutes before each timed event in the next day, once per event. */
function scheduleReminders(): void {
  window.clearTimeout(calReminderTimer);
  const tick = () => {
    const now = Date.now();
    const list = Object.values(S.getState().calendar.events);
    for (const e of list) {
      if (isAllDay(e) || e.status === "cancelled") continue;
      const startsIn = startOf(e).getTime() - now;
      if (startsIn > 0 && startsIn <= 10 * 60_000 && !remindedFor.has(calKey(e)) && endOf(e).getTime() > now) {
        remindedFor.add(calKey(e));
        const mins = Math.max(1, Math.round(startsIn / 60_000));
        const title = `${e.summary ?? "Event"} in ${mins} min`;
        toast(title, 6000);
        if (bridge.notify && S.getState().prefs.notifications !== "off") bridge.notify(title, e.location ?? (e.hangoutLink ? "Google Meet" : "Google Calendar")).catch(() => {});
      }
    }
    calReminderTimer = window.setTimeout(tick, 30_000);
  };
  tick();
}
/** The next timed event, for the top-bar chip. */
export const upcoming = (s: State): CalEvent | undefined => nextUp(Object.values(s.calendar.events).filter((e) => !s.calendar.hidden.includes(e.calendarId ?? "primary")));
export { startOfWeek };

// ---- Notes: Keep-style, one JSON document in Drive app data ----
let notesSaveTimer = 0;
export async function loadNotes(): Promise<void> {
  const n = S.getState().notes;
  if (n.loaded) return;
  try {
    const raw = await bridge.notesLoad();
    const parsed = raw ? (JSON.parse(raw) as { notes?: Note[] }) : { notes: [] };
    S.setState((s) => ({ notes: { ...s.notes, items: parsed.notes ?? [], loaded: true, error: undefined } }));
  } catch (e) {
    S.setState((s) => ({ notes: { ...s.notes, loaded: true, error: String(e) } }));
  }
}
function persistNotes(): void {
  window.clearTimeout(notesSaveTimer);
  notesSaveTimer = window.setTimeout(async () => {
    const items = S.getState().notes.items;
    S.setState((s) => ({ notes: { ...s.notes, saving: true } }));
    try { await bridge.notesSave(JSON.stringify({ v: 1, notes: items })); S.setState((s) => ({ notes: { ...s.notes, saving: false, error: undefined } })); }
    catch (e) { S.setState((s) => ({ notes: { ...s.notes, saving: false, error: String(e) } })); toast(`Notes did not save: ${String(e)}`); }
  }, 900);
}
export function addNote(title: string, body: string, color = "none"): string {
  const id = uuid();
  const note: Note = { id, title, body, color, pinned: false, updated: new Date().toISOString() };
  S.setState((s) => ({ notes: { ...s.notes, items: [note, ...s.notes.items] } }));
  persistNotes();
  return id;
}
export function updateNote(id: string, patchNote: Partial<Note>): void {
  S.setState((s) => ({ notes: { ...s.notes, items: s.notes.items.map((n) => (n.id === id ? { ...n, ...patchNote, updated: new Date().toISOString() } : n)) } }));
  persistNotes();
}
export function deleteNote(id: string): void {
  S.setState((s) => ({ notes: { ...s.notes, items: s.notes.items.filter((n) => n.id !== id), editing: s.notes.editing === id ? undefined : s.notes.editing } }));
  persistNotes();
  toast("Note deleted");
}
export function setNotesQuery(query: string): void {
  S.setState((s) => ({ notes: { ...s.notes, query } }));
}
export function editNote(id?: string): void {
  S.setState((s) => ({ notes: { ...s.notes, editing: id } }));
}

// ---- huddles: Google's own (a Meet behind a huddle message everyone on Chat sees), Confab voice as the fallback ----
let voice: Huddle | null = null;
let meetPoll = 0;
const NO_HUDDLE: Pick<State["huddle"], "kind" | "space" | "code" | "uri" | "message" | "active" | "participants" | "peers" | "speaking" | "error"> = { kind: undefined, space: undefined, code: undefined, uri: undefined, message: undefined, active: false, participants: [], peers: {}, speaking: [], error: undefined };

let starting = false;
/**
 * Starts a huddle the way Google Chat does: a new Meet, the huddle message in the space, the call in the browser,
 * the strip here. The strip and the header react on the click itself; a second click while Google answers is ignored.
 */
export async function startHuddle(space: string): Promise<void> {
  if (starting) return;
  // Only Google's own huddle rings someone on Google Chat, and no public API can start one. So a direct message or
  // group chat opens in Google Chat, where the Huddle button rings them; the huddle then shows here by its message.
  const sp = S.getState().spaces.find((x) => x.name === space);
  if (sp && sp.spaceType !== "SPACE" && sp.spaceUri && bridge.openHuddleWindow) {
    await openExternal(sp.spaceUri);
    toast(`Google Chat is opening. Press the call button at the top to ring ${spaceLabel(sp, S.getState().members[space], me())}.`, 6000);
    return;
  }
  starting = true;
  leaveHuddle();
  S.setState((s) => ({ huddle: { ...s.huddle, ...NO_HUDDLE, kind: "meet", space, starting: true } }));
  let meet;
  try {
    meet = await bridge.createMeet();
  } catch (e) {
    starting = false;
    const why = String(e);
    console.warn("createMeet", why);
    S.setState((s) => ({ huddle: { ...s.huddle, ...NO_HUDDLE, starting: false } }));
    toast(/403|PERMISSION|disabled|not been used|insufficient/i.test(why) ? "Google Meet API is off for this project; starting a Confab voice huddle instead" : `Meet did not answer (${why.slice(0, 80)}); starting a Confab voice huddle`, 4000);
    return joinVoice(space);
  }
  // The call opens as soon as the Meet exists; the message to the space follows in the background.
  joinHuddle(space, meet.meetingCode, meet.meetingUri);
  starting = false;
  try {
    const st = S.getState();
    // A mention is what makes Google notify a space's members; in a big space it would page everyone for one call.
    const members = st.members[space];
    const mentionAll = st.spaces.find((x) => x.name === space)?.spaceType === "SPACE" && !!members && members.length <= RING_SPACE_MAX;
    const posted = await bridge.postHuddle(space, meet.meetingUri, meet.meetingCode, mentionAll);
    putMessage(posted);
    S.setState((s) => (s.huddle.code === meet.meetingCode ? { huddle: { ...s.huddle, message: posted.name } } : {}));
  } catch (e) {
    console.warn("postHuddle", e);
    toast("The huddle is on, but the message to the space failed");
  }
}

const RING_SPACE_MAX = 20;
const RING_MS = 40_000;
const RING_FRESH_MS = 60_000;
let ringStop = () => {};

/** Someone started a huddle here: rings like a call. False when it should stay an ordinary message. */
function ringHuddle(space: string, m: Message): boolean {
  const s = S.getState();
  const link = huddleOf(m);
  if (!link || s.me.state === "dnd" || s.muted.includes(space) || s.huddle.code === link.code) return false;
  if (Date.now() - new Date(m.createTime ?? 0).getTime() > RING_FRESH_MS) return false;
  ringStop();
  const who = m.sender?.displayName ?? nameMapOf(s.members[space])[m.sender?.name ?? ""]?.displayName ?? "Someone";
  S.setState({ ringing: { space, code: link.code, uri: link.uri, message: m.name, from: who } });
  if (s.prefs.sounds) chime();
  const again = s.prefs.sounds ? window.setInterval(chime, 2500) : 0;
  const expire = window.setTimeout(() => { dismissRing(); toast(`Missed a huddle from ${who}`); }, RING_MS);
  ringStop = () => { window.clearInterval(again); window.clearTimeout(expire); ringStop = () => {}; };
  void bridge.raiseWindow?.().catch((e) => console.warn("raise", e));
  if (s.prefs.notifications !== "off" && bridge.notify) bridge.notify(`${who} started a huddle`, "Open Confab to join").catch((e) => console.warn("notify", e));
  return true;
}
/** Stops the ring. Google has no decline to send, so the caller sees nothing. */
export function dismissRing(): void {
  ringStop();
  S.setState({ ringing: undefined });
}
export function answerRing(): void {
  const r = S.getState().ringing;
  if (!r) return;
  dismissRing();
  openSpace(r.space);
  joinHuddle(r.space, r.code, r.uri, r.message);
}

/** The call in its mini window, or the browser where the shell has no such window. */
export async function openHuddleCall(uri: string, space?: string): Promise<void> {
  if (!bridge.openHuddleWindow) return openExternal(uri);
  const email = S.getState().auth.user?.email;
  const u = new URL(uri);
  if (email) u.searchParams.set("authuser", email);
  const name = S.getState().spaces.find((x) => x.name === space)?.displayName;
  try {
    await bridge.openHuddleWindow(u.toString(), name ? `Huddle in ${name}` : "Huddle", email);
  } catch (e) {
    console.warn("huddle window", e);
    toast("The call window did not open; using your browser");
    await openExternal(uri);
  }
}

/** Moves the running call to the browser, for what the mini window cannot do, such as sharing the screen. */
export function huddleToBrowser(): void {
  const uri = S.getState().huddle.uri;
  if (!uri) return;
  void bridge.closeHuddleWindow?.();
  void openExternal(uri);
}

/** Joins a huddle someone started (or my own): Meet opens in the mini window, the strip follows who is in. */
export function joinHuddle(space: string, code: string, uri: string, message?: string): void {
  if (S.getState().huddle.kind === "voice") leaveHuddle();
  void openHuddleCall(uri, space);
  S.setState((s) => ({ huddle: { ...s.huddle, ...NO_HUDDLE, kind: "meet", space, code, uri, message, participants: [], starting: false } }));
  window.clearInterval(meetPoll);
  let wasActive = false;
  let wasIn = false;
  const poll = async () => {
    const h = S.getState().huddle;
    if (h.kind !== "meet" || h.code !== code) { window.clearInterval(meetPoll); return; }
    try {
      const st = await bridge.meetStatus(code);
      if (S.getState().huddle.code !== code) return;
      S.setState((s) => ({ huddle: { ...s.huddle, active: st.active, participants: st.participants, error: undefined } }));
      if (st.active) wasActive = true;
      else if (wasActive) { leaveHuddle(); toast("The huddle ended"); return; }
      // Left from Meet's own button while others stay: the window has nothing more to show.
      const inCall = st.participants.some((p) => p.user === me());
      if (inCall) wasIn = true;
      else if (wasIn && bridge.closeHuddleWindow) leaveHuddle();
    } catch (e) {
      S.setState((s) => ({ huddle: { ...s.huddle, error: String(e).slice(0, 120) } }));
    }
  };
  void poll();
  meetPoll = window.setInterval(() => void poll(), 8000);
}

export function leaveHuddle(): void {
  window.clearInterval(meetPoll);
  if (S.getState().huddle.kind === "meet") void bridge.closeHuddleWindow?.();
  const v = voice;
  if (v) {
    voice = null;
    v.close();
    bridge.relaySend({ type: "huddle_leave", space: v.space });
    S.setState((s) => ({ huddles: { ...s.huddles, [v.space]: (s.huddles[v.space] ?? []).filter((u) => u !== me()) } }));
  }
  if (S.getState().huddle.kind) S.setState((s) => ({ huddle: { ...s.huddle, ...NO_HUDDLE, starting: false, muted: false, listenOnly: false } }));
}

/** Confab voice: in-app audio between Confab clients over the relay, for when Meet is not available. */
export async function joinVoice(space: string): Promise<void> {
  if (voice?.space === space) return;
  leaveHuddle();
  const meName = me();
  if (!meName) return;
  const h = new Huddle(space, {
    me: meName,
    send: (sp, to, payload) => bridge.relaySend({ type: "huddle_signal", space: sp, to, payload }),
    onPeers: (peers) => S.setState((s) => ({ huddle: { ...s.huddle, peers } })),
    onSpeaking: (speaking) => S.setState((s) => (sameList(s.huddle.speaking, speaking) ? {} : { huddle: { ...s.huddle, speaking } })),
  });
  voice = h;
  S.setState((s) => ({ huddle: { ...s.huddle, ...NO_HUDDLE, kind: "voice", space, active: true } }));
  await h.open();
  if (voice !== h) return;
  S.setState((s) => ({ huddle: { ...s.huddle, listenOnly: h.listenOnly } }));
  bridge.relaySend({ type: "huddle_join", space });
  h.members(S.getState().huddles[space] ?? []);
  if (h.listenOnly) toast("No microphone: you can listen, not talk");
}
export function toggleHuddleMute(): void {
  if (!voice) return;
  const on = !voice.muted;
  voice.setMuted(on);
  S.setState((s) => ({ huddle: { ...s.huddle, muted: on } }));
}
const sameList = (a: string[], b: string[]) => a.length === b.length && a.every((x, i) => x === b[i]);

// ---- link previews (consent first) ----
const hostOf = (url: string): string => { try { return new URL(url).hostname.replace(/^www\./, ""); } catch { return ""; } };
/** Allowed without asking: the preference says always, or the site was allowed before. */
export function previewAllowed(url: string): boolean {
  const s = S.getState();
  if (s.prefs.linkPreviews === "off") return false;
  if (s.prefs.linkPreviews === "on") return true;
  return s.previewDomains.includes(hostOf(url));
}
/** The user said yes: for this link, for its site from now on, or for every link. Then fetch. */
export function allowPreview(url: string, scope: "once" | "site" | "always"): void {
  if (scope === "site") S.setState((s) => ({ previewDomains: s.previewDomains.includes(hostOf(url)) ? s.previewDomains : [...s.previewDomains, hostOf(url)] }));
  if (scope === "always") setPrefs({ linkPreviews: "on" });
  void loadPreview(url);
}
export function forgetPreviewSite(host: string): void {
  S.setState((s) => ({ previewDomains: s.previewDomains.filter((h) => h !== host) }));
}
export async function loadPreview(url: string): Promise<void> {
  const s = S.getState();
  if (!bridge.unfurl || url in s.previews) return;
  S.setState((st) => ({ previews: { ...st.previews, [url]: { loading: true } } }));
  try {
    const p = await bridge.unfurl(url);
    S.setState((st) => ({ previews: { ...st.previews, [url]: p } }));
  } catch (e) {
    S.setState((st) => ({ previews: { ...st.previews, [url]: { error: String(e).replace(/^Error:\s*/, "") } } }));
  }
}
export const canPreview = (): boolean => !!bridge.unfurl;

// ---- notifications ----
/** A system notification for a message that landed out of view, by the Notifications preference. */
function maybeNotify(space: string, msg: Message, isOpen: boolean): void {
  const s = S.getState();
  if (isOpen || !bridge.notify || s.prefs.notifications === "off" || s.me.state === "dnd" || s.muted.includes(space)) return;
  const sp = s.spaces.find((x) => x.name === space);
  const mentioned = mentionsMe(msg, me());
  if (s.prefs.notifications === "mentions" && sp?.spaceType === "SPACE" && !mentioned) return;
  const names = nameMapOf(s.members[space]);
  const who = msg.sender?.displayName ?? "Someone";
  const label = sp ? spaceLabel(sp, s.members[space], me()) : space;
  const title = sp?.spaceType === "SPACE" ? `${who} in #${label}` : sp?.spaceType === "GROUP_CHAT" ? `${who} in ${label}` : who;
  const body = plainText(bodyOf(msg), (u) => names[u]?.displayName ?? u) || (msg.attachment?.length ? "Sent a file" : "New message");
  bridge.notify(title, body.length > 200 ? `${body.slice(0, 197)}…` : body).catch((e) => console.warn("notify", e));
}

/** The in-app file viewer; `open` is an http link for the Open button, `back` the archive an entry came from. */
export function openViewer(src: string, name: string, mime?: string, open?: string, back?: { src: string; name: string; mime?: string }): void {
  ui({ viewer: { src, name, mime, open, back } });
}
export function ui(p: Partial<State["ui"]>): void {
  S.setState((s) => ({ ui: { ...s.ui, ...p } }));
}

// ---- realtime ----
function pruneTyping(): void {
  const now = Date.now();
  const s = S.getState();
  let changed = false;
  const next: State["typing"] = {};
  for (const [space, users] of Object.entries(s.typing)) {
    const keep: Record<string, { until: number; thread?: string }> = {};
    for (const [u, e] of Object.entries(users)) if (e.until > now) keep[u] = e; else changed = true;
    if (Object.keys(keep).length) next[space] = keep;
  }
  if (changed) S.setState({ typing: next });
}

function handleFrame(m: ServerMsg | ShellEvent): void {
  try {
    route(m);
  } catch (e) {
    console.error("frame", m, e);
  }
}

function route(m: ServerMsg | ShellEvent): void {
  switch (m.type) {
    case "huddle_window_closed":
      leaveHuddle();
      return;
    case "account": {
      // The shell switched accounts (another window, a notification click): follow it.
      const cur = S.getState().accounts.find((a) => a.active)?.id;
      if (cur !== m.id) { resetForAccount(); void loadAccounts(); void rebootWorkspace(); }
      return;
    }
    case "relay_state": {
      S.setState({ relay: m.state });
      if (m.state === "open") subscribeRelay();
      return;
    }
    case "welcome": {
      S.setState((s) => {
        const presence = { ...s.presence };
        for (const u of m.online) if (!presence[u]) presence[u] = { state: "active" };
        return { presence, relay: "open", auth: s.auth.user ? s.auth : { status: "in", user: m.user } };
      });
      subscribeRelay();
      return;
    }
    case "typing": {
      if (m.user === me()) return;
      S.setState((s) => {
        const users = { ...(s.typing[m.space] ?? {}) };
        if (m.active) users[m.user] = { until: Date.now() + (m.ttl_ms || 5000), thread: m.thread };
        else delete users[m.user];
        return { typing: { ...s.typing, [m.space]: users } };
      });
      return;
    }
    case "presence": {
      S.setState((s) => ({ presence: { ...s.presence, [m.user]: { state: m.state, custom_status: m.custom_status } } }));
      return;
    }
    case "receipt": {
      if (m.user === me()) return;
      S.setState((s) => ({ receipts: { ...s.receipts, [m.space]: { ...(s.receipts[m.space] ?? {}), [m.user]: { message: m.message, at: m.at } } } }));
      return;
    }
    case "draft": {
      // Another device of mine typed this; take it only when this one has nothing. note: no merge.
      const key = draftKey(m.space, m.thread);
      S.setState((s) => (s.drafts[key] ? {} : { drafts: { ...s.drafts, [key]: m.text } }));
      return;
    }
    case "event":
      handleEvent(m.event);
      return;
    case "huddle": {
      S.setState((s) => ({ huddles: { ...s.huddles, [m.space]: m.users } }));
      if (voice?.space === m.space) voice.members(m.users);
      return;
    }
    case "huddle_signal": {
      if (voice?.space === m.space) void voice.signal(m.from, m.payload as HuddleSignalPayload);
      return;
    }
    case "outbox": {
      const s = S.getState();
      const all = Object.values(s.messages).flatMap((p) => p.items);
      const opt = all.find((x) => clientIdOf(x.name) === m.clientId || x.clientAssignedMessageId === `client-${m.clientId}`);
      if (m.status === "sent" && m.message) {
        const msg: Message = { ...m.message, clientAssignedMessageId: m.message.clientAssignedMessageId ?? `client-${m.clientId}` };
        delete msg._pending;
        if (opt && opt.name !== msg.name) dropMessage(opt.name);
        putMessage(msg);
      } else if (opt) {
        patchMessage(opt.name, (x) => ({ ...x, _pending: "failed", _error: m.error ?? "Not sent" }));
      }
      return;
    }
    case "error":
      console.warn("relay error", m.code, m.message);
      return;
    case "pong":
      return;
  }
}

function handleEvent(ev: ChatEvent): void {
  const t = ev.event_type.replace("google.workspace.chat.", "");
  const s = S.getState();
  if (t.startsWith("message.v1.")) {
    const msg = ev.message;
    if (!msg) return;
    if (t.endsWith("deleted")) {
      if (S.getState().ringing?.message === msg.name) dismissRing();
      tombstone(msg.name, msg);
      return;
    }
    const space = ev.space || msg.name.split("/").slice(0, 2).join("/");
    const wasStarted = (() => { const p = findMessage(msg.name); return !!p && isHuddleStart(p); })();
    const known = findMessage(msg.name) || (msg.clientAssignedMessageId && findMessage(`${space}/messages/${msg.clientAssignedMessageId}`));
    const clean = { ...msg };
    delete clean._pending;
    // A list that was never loaded stays unloaded; the message is kept so the unread preview is right.
    putMessage(clean);
    if (t.endsWith("updated")) {
      const ring = S.getState().ringing;
      // The caller gave up or the huddle ended before it was answered.
      if (ring?.message === msg.name && !isHuddleStart(msg)) { const from = ring.from; dismissRing(); toast(`Missed a huddle from ${from}`); }
      // Google may mark a message as a huddle only after creating it.
      else if (!wasStarted && msg.sender?.name !== me() && isHuddleStart(msg)) ringHuddle(space, msg);
    }
    if (t.endsWith("created") && !known && msg.sender?.name !== me()) {
      const isOpen = s.ui.activeSpace === space && document.hasFocus();
      if (!isOpen) {
        S.setState((st) => ({ unread: { ...st.unread, [space]: (st.unread[space] ?? 0) + 1 } }));
        if (s.prefs.sounds && s.me.state !== "dnd" && !s.muted.includes(space)) chime();
      }
      if (!(isHuddleStart(msg) && ringHuddle(space, msg))) maybeNotify(space, msg, isOpen);
      // A reply in the open thread does not count against the space either way.
    }
    return;
  }
  if (t.startsWith("reaction.v1.")) {
    const r = ev.reaction;
    const unicode = r?.emoji?.unicode;
    const name = r?.name?.split("/reactions/")[0];
    if (!r || !unicode || !name) return;
    const isMe = r.user?.name === me();
    const have = (s.myReactions[name] ?? []).includes(unicode);
    if (t.endsWith("created")) {
      if (isMe && have) return; // already applied optimistically
      if (isMe) setMine(name, unicode, true);
      patchMessage(name, (m) => ({ ...m, emojiReactionSummaries: bump(m.emojiReactionSummaries, unicode, 1) }));
    } else {
      if (isMe && !have) return;
      if (isMe) setMine(name, unicode, false);
      patchMessage(name, (m) => ({ ...m, emojiReactionSummaries: bump(m.emojiReactionSummaries, unicode, -1) }));
    }
    return;
  }
  if (t.startsWith("membership.v1.")) {
    const mem = ev.membership;
    if (!mem) return;
    S.setState((st) => {
      const list = st.members[ev.space];
      if (!list) return {};
      const next = t.endsWith("deleted") ? list.filter((x) => x.name !== mem.name) : [...list.filter((x) => x.name !== mem.name), mem];
      return { members: { ...st.members, [ev.space]: next } };
    });
    return;
  }
  if (t === "space.v1.updated" && ev.space_resource) {
    const sp = ev.space_resource;
    S.setState((st) => ({ spaces: st.spaces.map((x) => (x.name === sp.name ? { ...x, ...sp } : x)) }));
  }
}

export { byTime };

/** Dev only: feed a relay/shell frame as if the bridge delivered it. */
export function injectFrame(m: ServerMsg | ShellEvent): void {
  handleFrame(m);
}

// ---- Drive: the user's Drive, with incremental consent so the chat sign-in is never redone ----
const D = () => S.getState().drive;
const setDrive = (p: Partial<State["drive"]>) => S.setState((s) => ({ drive: { ...s.drive, ...p } }));
const pinKey = () => `prism.drivePins.${S.getState().auth.user?.email ?? "me"}`;
let driveSeq = 0;

export async function openDrive(): Promise<void> {
  try { setDrive({ pins: JSON.parse(localStorage.getItem(pinKey()) ?? "[]") as DriveFolder[] }); } catch { /* no pins yet */ }
  if (D().connected !== true) {
    try { setDrive({ connected: await bridge.driveStatus() }); } catch (e) { setDrive({ connected: false, error: String(e) }); }
  }
  if (!D().connected) return;
  void bridge.driveDrives().then((drives) => setDrive({ drives })).catch(() => {});
  await driveLoad();
}

/** Opens Google's consent page for Drive alone; the current sign-in stays as it is. */
export async function connectDrive(): Promise<void> {
  setDrive({ connecting: true, error: undefined });
  try {
    const ok = await bridge.driveConnect();
    setDrive({ connected: ok, connecting: false, error: ok ? undefined : "Drive access was not given." });
    if (ok) await openDrive();
  } catch (e) {
    setDrive({ connecting: false, error: String(e) });
  }
}

/** Loads the current place; `more` appends the next page. A newer load wins over a slower older one. */
export async function driveLoad(more = false): Promise<void> {
  const { where, nextPageToken } = D();
  if (where.kind === "drives") { setDrive({ items: [], nextPageToken: undefined, loading: false, error: undefined }); return; }
  const seq = ++driveSeq;
  setDrive({ loading: true, error: undefined, ...(more ? {} : { items: [], nextPageToken: undefined }) });
  try {
    const page = await bridge.driveList(where, more ? nextPageToken : undefined);
    if (seq !== driveSeq) return;
    setDrive({ items: more ? [...D().items, ...page.files] : page.files, nextPageToken: page.nextPageToken, loading: false });
  } catch (e) {
    if (seq !== driveSeq) return;
    const msg = String(e);
    // A sign-in that lost Drive access (revoked in the Google account) goes back to the connect card.
    if (/insufficient|PERMISSION_DENIED.*scope|403/i.test(msg) && /scope/i.test(msg)) setDrive({ connected: false, loading: false });
    else setDrive({ loading: false, error: msg });
  }
}

export function driveGo(where: DriveWhere, trail: DriveFolder[] = []): void {
  setDrive({ where, trail, query: where.kind === "search" ? D().query : "" });
  void driveLoad();
}

/** The folder new files and folders go into, when the current place has one. */
export function driveParent(): string | undefined {
  const w = D().where;
  return w.kind === "root" ? "root" : w.kind === "folder" ? w.id : undefined;
}

export function driveOpenFolder(f: DriveFolder): void {
  const { where, trail } = D();
  const base = where.kind === "folder" ? trail : [];
  const at = base.findIndex((t) => t.id === f.id);
  driveGo({ kind: "folder", id: f.id, driveId: f.driveId }, at >= 0 ? base.slice(0, at + 1) : [...base, f]);
}

/** Folders open in place; files open in the viewer when it can show them, else in Google Drive. */
export async function driveOpen(item: DriveItem): Promise<void> {
  if (isFolder(item)) { driveOpenFolder({ id: targetId(item), name: item.name, driveId: item.driveId }); return; }
  const google = isGoogleFile(item.mimeType);
  const mime = google ? "application/pdf" : item.mimeType;
  const name = google ? `${item.name}.pdf` : item.name;
  if (!previewKind(mime, name)) { if (item.webViewLink) void openExternal(item.webViewLink); return; }
  try {
    const url = await bridge.driveContentUrl(item.shortcutDetails ? { ...item, id: targetId(item), mimeType: item.shortcutDetails.targetMimeType ?? item.mimeType } : item);
    openViewer(url, name, mime, item.webViewLink);
  } catch (e) { toast(`Could not open ${item.name}: ${String(e)}`); }
}

export async function driveDownload(item: DriveItem): Promise<void> {
  try {
    const url = await bridge.driveContentUrl(item);
    await saveAttachment(url, isGoogleFile(item.mimeType) ? `${item.name}.pdf` : item.name);
  } catch (e) { toast(`Could not download: ${String(e)}`); }
}

/** Swaps the updated item into the list, or drops it when it no longer belongs in this place. */
function driveReplace(next: DriveItem): void {
  const w = D().where;
  const parent = w.kind === "root" ? "root" : w.kind === "folder" ? w.id : undefined;
  const gone = (w.kind === "trash" ? !next.trashed : next.trashed) || (w.kind === "starred" && !next.starred) || (!!parent && !next.parents?.includes(parent));
  setDrive({ items: gone ? D().items.filter((i) => i.id !== next.id) : D().items.map((i) => (i.id === next.id ? next : i)) });
}

async function driveUpdate(item: DriveItem, patchItem: { name?: string; starred?: boolean; trashed?: boolean }, done: string, move?: { add: string; remove: string }): Promise<void> {
  try { driveReplace(await bridge.driveUpdate(item.id, patchItem, move)); toast(done); }
  catch (e) { toast(`Drive said no: ${String(e)}`); }
}
export const driveRename = (item: DriveItem, name: string) => driveUpdate(item, { name }, `Renamed to ${name}`);
export const driveStar = (item: DriveItem) => driveUpdate(item, { starred: !item.starred }, item.starred ? "Removed from Starred" : "Added to Starred");
export const driveTrash = (item: DriveItem) => driveUpdate(item, { trashed: true }, `${item.name} moved to the bin`);
export const driveRestore = (item: DriveItem) => driveUpdate(item, { trashed: false }, `${item.name} restored`);
export const driveMove = (item: DriveItem, to: DriveFolder) => driveUpdate(item, {}, `Moved to ${to.name}`, { add: to.id, remove: (item.parents ?? []).join(",") });

export async function driveNewFolder(name: string): Promise<void> {
  const parent = driveParent();
  if (!parent) return;
  try {
    const f = await bridge.driveCreateFolder(name, parent);
    setDrive({ items: [f, ...D().items] });
    toast(`Created ${name}`);
  } catch (e) { toast(`Could not create the folder: ${String(e)}`); }
}

async function driveUploading<T>(n: number, work: () => Promise<T>): Promise<T | undefined> {
  setDrive({ uploading: D().uploading + n });
  try { return await work(); } catch (e) { toast(`Upload failed: ${String(e)}`); return undefined; }
  finally { setDrive({ uploading: Math.max(0, D().uploading - n) }); }
}
export async function driveUploadFiles(files: File[]): Promise<void> {
  const parent = driveParent();
  if (!parent) { toast("Open My Drive or a folder to upload into it"); return; }
  for (const f of files) {
    const up = await driveUploading(1, () => bridge.driveUpload(f, parent));
    if (up && driveParent() === parent) setDrive({ items: [up, ...D().items] });
  }
  if (files.length) toast(files.length === 1 ? `Uploaded ${files[0]!.name}` : `Uploaded ${files.length} files`);
}
export async function driveUploadPaths(paths: string[]): Promise<void> {
  const parent = driveParent();
  if (!parent || !bridge.driveUploadPath) { toast("Open My Drive or a folder to upload into it"); return; }
  for (const p of paths) {
    const up = await driveUploading(1, () => bridge.driveUploadPath!(p, parent));
    if (up && driveParent() === parent) setDrive({ items: [up, ...D().items] });
  }
}

/** Pinned folders live on this device, per account. */
export function driveTogglePin(f: DriveFolder): void {
  const was = D().pins.some((p) => p.id === f.id);
  const pins = was ? D().pins.filter((p) => p.id !== f.id) : [...D().pins, f];
  setDrive({ pins });
  try { localStorage.setItem(pinKey(), JSON.stringify(pins)); } catch { /* storage full or blocked: pins last this session */ }
  toast(was ? `Unpinned ${f.name}` : `Pinned ${f.name}`);
}

export async function driveCopyLink(item: DriveItem): Promise<void> {
  if (!item.webViewLink) return;
  try { await navigator.clipboard.writeText(item.webViewLink); toast("Link copied"); } catch { toast("Could not copy the link"); }
}

export function driveUi(p: Partial<Pick<State["drive"], "renaming" | "moving" | "newFolder" | "query">>): void {
  setDrive(p);
}
