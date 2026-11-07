// Browser shell: fetch straight to the Chat API (twin or Google) and a WebSocket to the relay.
// note: browser mode is a dev convenience; tokens live in the Rust core on desktop. Here the
// access token sits in sessionStorage, there is no refresh, and a 401 simply signs you out.
import type {
  Account, Attachment, AuthStatus, Availability, Bridge, CalEvent, CalendarEntry, ClientMsg, EventsPage, MeetSpace, MeetStatus, Membership, Message, Reaction, ServerMsg, ShellEvent, Space, Task, User,
} from "../bridge-types";
import { uuid } from "../lib/ids";
import { personToUser, type Person } from "../lib/people";

const E = import.meta.env;
const twin = E.VITE_PRISM_TWIN_BASE ?? "http://127.0.0.1:8081";
export const endpoints = {
  chatApi: E.VITE_PRISM_CHAT_API ?? twin,
  // Google Tasks lives on its own host; the twin serves it on the same base as Chat.
  tasksApi: E.VITE_PRISM_TASKS_API ?? ((E.VITE_PRISM_CHAT_API ?? twin) === twin ? twin : "https://tasks.googleapis.com"),
  meetApi: E.VITE_PRISM_MEET_API ?? ((E.VITE_PRISM_CHAT_API ?? twin) === twin ? twin : "https://meet.googleapis.com"),
  calendarApi: E.VITE_PRISM_CALENDAR_API ?? ((E.VITE_PRISM_CHAT_API ?? twin) === twin ? twin : "https://www.googleapis.com"),
  driveApi: E.VITE_PRISM_DRIVE_API ?? ((E.VITE_PRISM_CHAT_API ?? twin) === twin ? twin : "https://www.googleapis.com"),
  oauthAuth: E.VITE_PRISM_OAUTH_AUTH ?? `${twin}/o/oauth2/v2/auth`,
  oauthToken: E.VITE_PRISM_OAUTH_TOKEN ?? `${twin}/token`,
  userinfo: E.VITE_PRISM_USERINFO ?? `${twin}/oauth2/v3/userinfo`,
  relayWs: E.VITE_PRISM_RELAY_WS ?? "ws://127.0.0.1:8080/ws",
  clientId: E.VITE_PRISM_CLIENT_ID ?? "prism-dev",
  twinUser: E.VITE_PRISM_TWIN_USER as string | undefined,
};
const SCOPES = [
  "openid", "email", "profile",
  "https://www.googleapis.com/auth/chat.spaces.readonly",
  "https://www.googleapis.com/auth/chat.memberships.readonly",
  "https://www.googleapis.com/auth/chat.messages",
  "https://www.googleapis.com/auth/chat.messages.reactions",
  "https://www.googleapis.com/auth/chat.users.readstate",
  "https://www.googleapis.com/auth/chat.spaces.create",
  "https://www.googleapis.com/auth/directory.readonly",
];
const TOKEN_KEY = "prism.token";
const VERIFIER_KEY = "prism.pkce";
const DEVICE_KEY = "prism.device";

type Listener = (m: ServerMsg | ShellEvent) => void;

function b64url(bytes: ArrayBuffer | Uint8Array): string {
  const arr = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
  let s = "";
  arr.forEach((b) => (s += String.fromCharCode(b)));
  return btoa(s).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function deviceId(): string {
  let id = localStorage.getItem(DEVICE_KEY);
  if (!id) localStorage.setItem(DEVICE_KEY, (id = uuid()));
  return id;
}

class ApiError extends Error {
  constructor(public status: number, message: string) {
    super(message);
  }
}

/** The browser's own Notification API; silently nothing until the user has allowed it. */
export async function webNotify(title: string, body: string): Promise<void> {
  if (typeof Notification === "undefined" || Notification.permission !== "granted") return;
  const n = new Notification(title, { body, tag: "prism", silent: true });
  n.onclick = () => { window.focus(); n.close(); };
}

export function createBrowserBridge(): Bridge {
  const listeners = new Set<Listener>();
  const emit = (m: ServerMsg | ShellEvent) => listeners.forEach((l) => l(m));
  let token: string | null = sessionStorage.getItem(TOKEN_KEY);
  let me: User | null = null;

  // ---- HTTP ----
  async function api<T>(method: string, path: string, body?: unknown, raw?: BodyInit, base = endpoints.chatApi): Promise<T> {
    if (!token) throw new ApiError(401, "Not signed in");
    const headers: Record<string, string> = { Authorization: `Bearer ${token}` };
    if (body !== undefined) headers["Content-Type"] = "application/json";
    const res = await fetch(`${base}${path}`, {
      method,
      headers,
      body: raw ?? (body !== undefined ? JSON.stringify(body) : undefined),
    });
    if (res.status === 401) {
      token = null;
      sessionStorage.removeItem(TOKEN_KEY);
      closeSocket();
    }
    if (!res.ok) {
      let msg = `${res.status} ${res.statusText}`;
      try {
        const j = (await res.json()) as { error?: { message?: string } };
        if (j.error?.message) msg = j.error.message;
      } catch { /* not JSON */ }
      throw new ApiError(res.status, msg);
    }
    if (res.status === 204) return undefined as T;
    const text = await res.text();
    return (text ? JSON.parse(text) : {}) as T;
  }

  async function userinfo(): Promise<User | null> {
    if (!token) return null;
    const res = await fetch(endpoints.userinfo, { headers: { Authorization: `Bearer ${token}` } });
    if (!res.ok) {
      if (res.status === 401 || res.status === 403) {
        token = null;
        sessionStorage.removeItem(TOKEN_KEY);
      }
      return null;
    }
    const u = (await res.json()) as { sub: string; name?: string; email?: string; picture?: string };
    me = { name: `users/${u.sub}`, displayName: u.name, email: u.email, avatarUrl: u.picture, type: "HUMAN" };
    return me;
  }

  // ---- OAuth code + PKCE in the page ----
  async function finishCallback(): Promise<void> {
    if (window.location.pathname !== "/callback") return;
    const q = new URLSearchParams(window.location.search);
    const code = q.get("code");
    const verifier = sessionStorage.getItem(VERIFIER_KEY);
    window.history.replaceState({}, "", "/");
    if (!code || !verifier) return;
    const form = new URLSearchParams({
      grant_type: "authorization_code",
      code,
      code_verifier: verifier,
      redirect_uri: `${window.location.origin}/callback`,
      client_id: endpoints.clientId,
    });
    const res = await fetch(endpoints.oauthToken, { method: "POST", body: form });
    if (!res.ok) throw new Error(`Token exchange failed: ${res.status}`);
    const t = (await res.json()) as { access_token: string };
    token = t.access_token;
    sessionStorage.setItem(TOKEN_KEY, token);
    sessionStorage.removeItem(VERIFIER_KEY);
  }
  const callbackDone = finishCallback().catch((e) => console.error(e));

  async function authStart(): Promise<AuthStatus> {
    const verifier = b64url(crypto.getRandomValues(new Uint8Array(32)));
    const challenge = b64url(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier)));
    sessionStorage.setItem(VERIFIER_KEY, verifier);
    const p = new URLSearchParams({
      response_type: "code",
      client_id: endpoints.clientId,
      redirect_uri: `${window.location.origin}/callback`,
      scope: SCOPES.join(" "),
      code_challenge: challenge,
      code_challenge_method: "S256",
      state: uuid(),
      access_type: "offline",
      prompt: "consent",
    });
    if (endpoints.twinUser) p.set("twin_user", endpoints.twinUser);
    window.location.assign(`${endpoints.oauthAuth}?${p.toString()}`);
    return new Promise(() => {}); // the page navigates away
  }

  // ---- Relay socket ----
  let ws: WebSocket | null = null;
  let backoff = 1000;
  let reconnectTimer: number | null = null;
  let wanted = false;
  let pingTimer: number | null = null;

  function closeSocket() {
    wanted = false;
    if (reconnectTimer) window.clearTimeout(reconnectTimer);
    if (pingTimer) window.clearInterval(pingTimer);
    reconnectTimer = pingTimer = null;
    ws?.close();
    ws = null;
  }
  function connect() {
    if (!token || ws) return;
    const tok = token;
    wanted = true;
    emit({ type: "relay_state", state: "connecting" });
    const sock = new WebSocket(endpoints.relayWs);
    ws = sock;
    sock.onopen = () => {
      sock.send(JSON.stringify({ type: "hello", token: tok, device_id: deviceId(), client_version: "0.1.0-web" } satisfies ClientMsg));
      backoff = 1000;
    };
    sock.onmessage = (ev) => {
      let m: ServerMsg;
      try { m = JSON.parse(String(ev.data)) as ServerMsg; } catch { return; }
      if (m.type === "welcome") {
        emit({ type: "relay_state", state: "open" });
        if (!pingTimer) pingTimer = window.setInterval(() => send({ type: "ping", t: Date.now() }), 25_000);
      }
      emit(m);
    };
    sock.onclose = () => {
      if (ws === sock) ws = null;
      if (pingTimer) window.clearInterval(pingTimer);
      pingTimer = null;
      emit({ type: "relay_state", state: "closed" });
      if (wanted && token) {
        reconnectTimer = window.setTimeout(connect, backoff);
        backoff = Math.min(backoff * 2, 30_000);
      }
    };
    sock.onerror = () => sock.close();
  }
  function send(msg: ClientMsg) {
    if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
  }

  // ---- Chat API ----
  const myId = () => me?.name ?? "users/me";

  const bridge: Bridge = {
    async authStatus() {
      await callbackDone;
      const user = await userinfo();
      if (user) connect();
      return user ? { signedIn: true, user } : { signedIn: false };
    },
    authStart,
    async signOut() {
      closeSocket();
      token = null;
      me = null;
      sessionStorage.removeItem(TOKEN_KEY);
    },
    async listSpaces() {
      const out: Space[] = [];
      let pageToken = "";
      do {
        const r = await api<{ spaces?: Space[]; nextPageToken?: string }>(
          "GET", `/v1/spaces?pageSize=100&pageToken=${encodeURIComponent(pageToken)}`,
        );
        out.push(...(r.spaces ?? []));
        pageToken = r.nextPageToken ?? "";
      } while (pageToken);
      return out;
    },
    async listMembers(space) {
      const r = await api<{ memberships?: Membership[] }>("GET", `/v1/${space}/members?pageSize=100`);
      return r.memberships ?? [];
    },
    async listMessages(space, opts = {}) {
      const q = new URLSearchParams({
        pageSize: String(opts.pageSize ?? 50),
        orderBy: "createTime desc",
        pageToken: opts.pageToken ?? "",
        filter: opts.thread ? `thread.name = "${opts.thread}"` : "",
      });
      const r = await api<{ messages?: Message[]; nextPageToken?: string }>("GET", `/v1/${space}/messages?${q}`);
      return r.nextPageToken ? { items: r.messages ?? [], nextPageToken: r.nextPageToken } : { items: r.messages ?? [] };
    },
    async sendMessage(space, text, opts = {}) {
      const clientId = opts.clientId ?? uuid();
      const optimistic: Message = {
        name: `${space}/messages/client-${clientId}`,
        sender: me ?? { name: myId() },
        createTime: new Date().toISOString(),
        text,
        clientAssignedMessageId: `client-${clientId}`,
        _pending: "sending",
        ...(opts.thread ? { thread: { name: opts.thread }, threadReply: true } : {}),
        ...(opts.attachments?.length ? { attachment: opts.attachments } : {}),
      };
      const body: Record<string, unknown> = { text };
      if (opts.thread) body.thread = { name: opts.thread };
      if (opts.attachments?.length) body.attachment = opts.attachments.map((a) => ({ attachmentDataRef: a.attachmentDataRef }));
      const q = `messageId=client-${clientId}&messageReplyOption=REPLY_MESSAGE_FALLBACK_TO_NEW_THREAD`;
      api<Message>("POST", `/v1/${space}/messages?${q}`, body)
        .then((message) => emit({ type: "outbox", clientId, status: "sent", message }))
        .catch((e: Error) => emit({ type: "outbox", clientId, status: "failed", error: e.message }));
      return optimistic;
    },
    editMessage: (name, text) => api<Message>("PATCH", `/v1/${name}?updateMask=text`, { text }),
    deleteMessage: (name, force) => api<void>("DELETE", `/v1/${name}${force ? "?force=true" : ""}`),
    async toggleReaction(messageName, unicode) {
      const filter = encodeURIComponent(`user.name = "${myId()}"`);
      const r = await api<{ reactions?: Reaction[] }>("GET", `/v1/${messageName}/reactions?filter=${filter}`);
      const mine = (r.reactions ?? []).find((x) => x.emoji?.unicode === unicode);
      if (mine?.name) await api<void>("DELETE", `/v1/${mine.name}`);
      else await api<Reaction>("POST", `/v1/${messageName}/reactions`, { emoji: { unicode } });
    },
    async markRead(space) {
      await api<unknown>("PATCH", `/v1/users/me/${space}/spaceReadState?updateMask=lastReadTime`, {
        lastReadTime: new Date().toISOString(),
      });
    },
    async search(query) {
      const r = await api<{ messages?: Message[] }>(
        "GET", `/v1/spaces/-/messages:search?query=${encodeURIComponent(query)}&pageSize=50`,
      );
      return r.messages ?? [];
    },
    async searchPeople(query) {
      const q = query.trim();
      if (!q) return [];
      const p = new URLSearchParams({ query: q, readMask: "names,emailAddresses,photos", sources: "DIRECTORY_SOURCE_TYPE_DOMAIN_PROFILE", pageSize: "20" });
      const r = await api<{ people?: Person[] }>("GET", `/v1/people:searchDirectoryPeople?${p}`);
      return (r.people ?? []).map(personToUser);
    },
    setupDm: (user) =>
      api<Space>("POST", "/v1/spaces:setup", { space: { spaceType: "DIRECT_MESSAGE" }, memberships: [{ member: { name: user, type: "HUMAN" } }] }),
    setupGroup: (members) =>
      api<Space>("POST", "/v1/spaces:setup", {
        space: { spaceType: "GROUP_CHAT" },
        memberships: members.map((name) => ({ member: { name, type: "HUMAN" } })),
      }),
    createSpace: (displayName, members) =>
      api<Space>("POST", "/v1/spaces:setup", {
        space: { spaceType: "SPACE", displayName },
        memberships: members.map((name) => ({ member: { name, type: "HUMAN" } })),
      }),
    async addMembers(space, users) {
      const out: Membership[] = [];
      for (const u of users) out.push(await api<Membership>("POST", `/v1/${space}/members`, { member: { name: u, type: "HUMAN" } }));
      return out;
    },
    async leaveSpace(space) {
      const mine = await api<Membership>("GET", `/v1/${space}/members/${myId()}`);
      await api<void>("DELETE", `/v1/${mine.name}`);
    },
    async uploadAttachment(space, file) {
      const form = new FormData();
      form.append("metadata", new Blob([JSON.stringify({ filename: file.name })], { type: "application/json" }));
      form.append("file", file, file.name);
      const r = await api<{ attachmentDataRef?: Attachment["attachmentDataRef"] }>(
        "POST", `/upload/v1/${space}/attachments:upload?uploadType=multipart`, undefined, form,
      );
      return { attachmentDataRef: r.attachmentDataRef, contentName: file.name, contentType: file.type, source: "UPLOADED_CONTENT" };
    },
    getAttachment: (name) => api<Attachment>("GET", `/v1/${name}`),
    async openUrl(url) {
      window.open(url, "_blank", "noopener,noreferrer");
    },
    async listTasks() {
      const r = await api<{ items?: Task[] }>("GET", "/tasks/v1/lists/@default/tasks?showCompleted=true&showHidden=false&maxResults=100", undefined, undefined, endpoints.tasksApi);
      return r.items ?? [];
    },
    addTask: (title, notes, due) => api<Task>("POST", "/tasks/v1/lists/@default/tasks", { title, ...(notes ? { notes } : {}), ...(due ? { due } : {}) }, undefined, endpoints.tasksApi),
    setTaskDone: (id, done) => api<Task>("PATCH", `/tasks/v1/lists/@default/tasks/${encodeURIComponent(id)}`, done ? { status: "completed" } : { status: "needsAction", completed: null }, undefined, endpoints.tasksApi),
    deleteTask: (id) => api<void>("DELETE", `/tasks/v1/lists/@default/tasks/${encodeURIComponent(id)}`, undefined, undefined, endpoints.tasksApi),
    notify: webNotify,
    // One account per browser session; the shell is where several live side by side.
    async accountsList(): Promise<Account[]> { return me ? [{ id: "browser", email: me.email, displayName: me.displayName, photo: me.avatarUrl, signedIn: true, active: true, unread: 0 }] : []; },
    async accountSwitch(id) { return { id, signedIn: !!me, active: true, unread: 0 }; },
    async accountAdd() { throw new Error("Several accounts need the desktop app"); },
    async accountRemove() { throw new Error("Several accounts need the desktop app"); },
    availabilityGet: () => api<Availability>("GET", "/v1/users/me/availability"),
    availabilityMark: (state, ttlSeconds) =>
      state === "away" ? api<Availability>("POST", "/v1/users/me/availability:markAsAway", {})
      : state === "dnd" ? api<Availability>("POST", "/v1/users/me/availability:markAsDoNotDisturb", { ttl: `${ttlSeconds ?? 3600}s` })
      : api<Availability>("POST", "/v1/users/me/availability:markAsActive", ttlSeconds ? { ttl: `${ttlSeconds}s` } : {}),
    availabilityStatus: (text, emoji, ttlSeconds) =>
      api<Availability>("PATCH", "/v1/users/me/availability?updateMask=customStatus", { customStatus: text ? { text, emoji: { unicode: emoji || "💬" }, ...(ttlSeconds ? { ttl: `${ttlSeconds}s` } : {}) } : {} }),
    async calendarList() {
      const r = await api<{ items?: CalendarEntry[] }>("GET", "/calendar/v3/users/me/calendarList?minAccessRole=reader", undefined, undefined, endpoints.calendarApi);
      return r.items ?? [];
    },
    async calendarEvents(calendarId, o) {
      const q = new URLSearchParams({ singleEvents: "true", maxResults: "250", showDeleted: "true" });
      if (o.syncToken) q.set("syncToken", o.syncToken);
      else { if (o.timeMin) q.set("timeMin", o.timeMin); if (o.timeMax) q.set("timeMax", o.timeMax); q.set("orderBy", "startTime"); }
      const items: CalEvent[] = [];
      let token: string | undefined;
      let page: string | undefined;
      do {
        if (page) q.set("pageToken", page);
        let r: EventsPage;
        try { r = await api<EventsPage>("GET", `/calendar/v3/calendars/${encodeURIComponent(calendarId)}/events?${q}`, undefined, undefined, endpoints.calendarApi); }
        catch (e) { if ((e as ApiError).status === 410) throw new Error("sync_expired"); throw e; }
        items.push(...(r.items ?? []).map((ev) => ({ ...ev, calendarId })));
        token = r.nextSyncToken ?? token;
        page = r.nextPageToken;
      } while (page);
      return { items, nextSyncToken: token };
    },
    calendarCreateEvent: (calendarId, ev) => {
      const body: Record<string, unknown> = { summary: ev.summary, description: ev.description, location: ev.location, start: ev.start, end: ev.end, attendees: ev.attendees.map((email) => ({ email })), ...(ev.recurrence?.length ? { recurrence: ev.recurrence } : {}) };
      if (ev.addMeet) body.conferenceData = { createRequest: { requestId: uuid(), conferenceSolutionKey: { type: "hangoutsMeet" } } };
      return api<CalEvent>("POST", `/calendar/v3/calendars/${encodeURIComponent(calendarId)}/events?sendUpdates=all${ev.addMeet ? "&conferenceDataVersion=1" : ""}`, body, undefined, endpoints.calendarApi).then((e) => ({ ...e, calendarId }));
    },
    calendarUpdateEvent: (calendarId, eventId, ev) => {
      const body: Record<string, unknown> = { summary: ev.summary, description: ev.description, location: ev.location, start: ev.start, end: ev.end, attendees: ev.attendees.map((email) => ({ email })), ...(ev.recurrence?.length ? { recurrence: ev.recurrence } : {}) };
      if (ev.addMeet) body.conferenceData = { createRequest: { requestId: uuid(), conferenceSolutionKey: { type: "hangoutsMeet" } } };
      return api<CalEvent>("PATCH", `/calendar/v3/calendars/${encodeURIComponent(calendarId)}/events/${encodeURIComponent(eventId)}?sendUpdates=all${ev.addMeet ? "&conferenceDataVersion=1" : ""}`, body, undefined, endpoints.calendarApi).then((e) => ({ ...e, calendarId }));
    },
    async calendarRespond(calendarId, eventId, response) {
      const ev = await api<CalEvent>("GET", `/calendar/v3/calendars/${encodeURIComponent(calendarId)}/events/${encodeURIComponent(eventId)}`, undefined, undefined, endpoints.calendarApi);
      const attendees = (ev.attendees ?? []).map((a) => (a.self ? { ...a, responseStatus: response } : a));
      return api<CalEvent>("PATCH", `/calendar/v3/calendars/${encodeURIComponent(calendarId)}/events/${encodeURIComponent(eventId)}?sendUpdates=all`, { attendees }, undefined, endpoints.calendarApi).then((e) => ({ ...e, calendarId }));
    },
    calendarDeleteEvent: (calendarId, eventId) => api<void>("DELETE", `/calendar/v3/calendars/${encodeURIComponent(calendarId)}/events/${encodeURIComponent(eventId)}?sendUpdates=all`, undefined, undefined, endpoints.calendarApi),
    async notesLoad() {
      const r = await api<{ files?: { id: string }[] }>("GET", "/drive/v3/files?spaces=appDataFolder&q=name%3D%27prism-notes.json%27&fields=files(id,name,modifiedTime)", undefined, undefined, endpoints.driveApi);
      const f = r.files?.[0];
      if (!f) return null;
      const res = await fetch(`${endpoints.driveApi}/drive/v3/files/${f.id}?alt=media`, { headers: { Authorization: `Bearer ${token}` } });
      return res.ok ? res.text() : null;
    },
    async notesSave(json) {
      const r = await api<{ files?: { id: string }[] }>("GET", "/drive/v3/files?spaces=appDataFolder&q=name%3D%27prism-notes.json%27&fields=files(id)", undefined, undefined, endpoints.driveApi);
      const f = r.files?.[0];
      const up = endpoints.driveApi + "/upload/drive/v3";
      if (f) { await fetch(`${up}/files/${f.id}?uploadType=media`, { method: "PATCH", headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" }, body: json }); return; }
      const boundary = "prism" + uuid();
      const body = `--${boundary}\r\nContent-Type: application/json\r\n\r\n${JSON.stringify({ name: "prism-notes.json", parents: ["appDataFolder"] })}\r\n--${boundary}\r\nContent-Type: application/json\r\n\r\n${json}\r\n--${boundary}--`;
      await fetch(`${up}/files?uploadType=multipart`, { method: "POST", headers: { Authorization: `Bearer ${token}`, "Content-Type": `multipart/related; boundary=${boundary}` }, body });
    },
    createMeet: () => api<MeetSpace>("POST", "/v2/spaces", { config: { accessType: "TRUSTED" } }, undefined, endpoints.meetApi),
    async meetStatus(code): Promise<MeetStatus> {
      const sp = await api<{ activeConference?: { conferenceRecord?: string } }>("GET", `/v2/spaces/${encodeURIComponent(code)}`, undefined, undefined, endpoints.meetApi);
      const rec = sp.activeConference?.conferenceRecord;
      if (!rec) return { active: false, participants: [] };
      const r = await api<{ participants?: { signedinUser?: { user?: string; displayName?: string }; anonymousUser?: { displayName?: string }; phoneUser?: { displayName?: string }; earliestStartTime?: string; latestEndTime?: string }[] }>("GET", `/v2/${rec}/participants?pageSize=100`, undefined, undefined, endpoints.meetApi);
      const participants = (r.participants ?? []).filter((p) => !p.latestEndTime).map((p) => ({ user: p.signedinUser?.user, displayName: p.signedinUser?.displayName ?? p.anonymousUser?.displayName ?? p.phoneUser?.displayName, since: p.earliestStartTime }));
      return { active: true, conferenceRecord: rec, participants };
    },
    postHuddle: (space, meetingUri, meetingCode) =>
      api<Message>("POST", `/v1/${space}/messages`, { text: "", annotations: [{ type: "RICH_LINK", startIndex: 0, length: 0, richLinkMetadata: { uri: meetingUri, richLinkType: "MEET_SPACE", meetSpaceLinkData: { meetingCode, type: "HUDDLE", huddleStatus: "STARTED" } } }] }),
    relaySend: send,
    onMessage(cb) {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
  };
  return bridge;
}
