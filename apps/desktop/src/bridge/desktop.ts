import { UserAttentionType, getCurrentWindow } from "@tauri-apps/api/window";
// Tauri shell: every call is an `invoke`; relay frames and shell events arrive as Tauri events.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import type { Account, Attachment, AuthStatus, Availability, Bridge, CalEvent, CalendarEntry, ClientMsg, EventsPage, LinkPreview, MeetSpace, MeetStatus, Membership, Message, Page, ServerMsg, ShellEvent, Space, Task, User } from "../bridge-types";

type Listener = (m: ServerMsg | ShellEvent) => void;

/** Standard base64 of the UTF-8 bytes of `s`: IPC headers must be ASCII, filenames need not be. Names are short. */
function b64utf8(s: string): string {
  let bin = "";
  for (const b of new TextEncoder().encode(s)) bin += String.fromCharCode(b);
  return btoa(bin);
}

export function createDesktopBridge(): Bridge {
  const listeners = new Set<Listener>();
  let attached = false;
  const attach = () => {
    if (attached) return;
    attached = true;
    void listen<ServerMsg>("relay", (e) => listeners.forEach((l) => l(e.payload)));
    void listen<ShellEvent>("shell", (e) => listeners.forEach((l) => l(e.payload)));
  };

  // One webview drag-drop registration, shared by onFileDrop (paths on drop) and onFileDrag (hover state).
  const dropCbs = new Set<(paths: string[]) => void>();
  const dragCbs = new Set<(over: boolean) => void>();
  let dragAttached = false;
  const attachDragDrop = () => {
    if (dragAttached) return;
    dragAttached = true;
    getCurrentWebview()
      .onDragDropEvent((e) => {
        const p = e.payload;
        if (p.type === "enter") dragCbs.forEach((cb) => cb(true));
        else if (p.type === "leave") dragCbs.forEach((cb) => cb(false));
        else if (p.type === "drop") {
          dragCbs.forEach((cb) => cb(false));
          if (p.paths.length) dropCbs.forEach((cb) => cb(p.paths));
        }
        // "over" only moves the cursor; nothing to report.
      })
      .catch(() => {
        dragAttached = false; // let the next subscriber retry
      });
  };

  return {
    authStatus: () => invoke<AuthStatus>("auth_status"),
    authStart: () => invoke<AuthStatus>("auth_start"),
    signOut: () => invoke<void>("sign_out"),
    listSpaces: () => invoke<Space[]>("list_spaces"),
    listMembers: (space) => invoke<Membership[]>("list_members", { space }),
    listMessages: (space, opts = {}) =>
      invoke<Page<Message>>("list_messages", {
        space,
        pageToken: opts.pageToken ?? null,
        thread: opts.thread ?? null,
        pageSize: opts.pageSize ?? null,
      }),
    sendMessage: (space, text, opts = {}) =>
      invoke<Message>("send_message", {
        space,
        text,
        thread: opts.thread ?? null,
        clientId: opts.clientId ?? null,
        // note: `attachments` is an addition to the table in docs/ARCHITECTURE.md; the core may ignore it until it supports uploads.
        attachments: opts.attachments ?? null,
      }),
    editMessage: (name, text) => invoke<Message>("edit_message", { name, text }),
    openHuddleWindow: (url, title, account) => invoke<void>("huddle_window_open", { url, title, account: account ?? null }),
    closeHuddleWindow: () => invoke<void>("huddle_window_close"),
    raiseWindow: async () => {
      const w = getCurrentWindow();
      await w.show();
      await w.setFocus();
      await w.requestUserAttention(UserAttentionType.Critical);
    },
    deleteMessage: (name, force) => invoke<void>("delete_message", { name, force: !!force }),
    toggleReaction: (messageName, unicode) => invoke<void>("toggle_reaction", { messageName, unicode }),
    markRead: (space, message, thread) => invoke<void>("mark_read", { space, message, thread: thread ?? null }),
    search: (query) => invoke<Message[]>("search", { query }),
    searchPeople: (query) => invoke<User[]>("search_people", { query }),
    setupDm: (user) => invoke<Space>("setup_dm", { user }),
    setupGroup: (members) => invoke<Space>("setup_group", { members }),
    createSpace: (displayName, members) => invoke<Space>("create_space", { displayName, members }),
    addMembers: (space, users) => invoke<Membership[]>("add_members", { space, users }),
    leaveSpace: (space) => invoke<void>("leave_space", { space }),
    uploadAttachment: async (space, file) => {
      // A Tauri drop hands over a path; a webview <input type=file> never does, so those bytes go over IPC
      // as the raw request body, with the metadata in headers (the Rust side reads them off the Request).
      const path = (file as File & { path?: string }).path;
      if (path) return invoke<Attachment>("upload_attachment", { space, path });
      const buf = new Uint8Array(await file.arrayBuffer());
      return invoke<Attachment>("upload_attachment_bytes", buf, {
        headers: {
          "x-prism-space": space,
          "x-prism-name": b64utf8(file.name),
          "x-prism-mime": file.type || "application/octet-stream",
        },
      });
    },
    uploadAttachmentPath: (space, path) => invoke<Attachment>("upload_attachment", { space, path }),
    getAttachment: (name) => invoke<Attachment>("get_attachment", { name }),
    openUrl: (url, account) => invoke<void>("open_url", { url, account: account ?? null }),
    saveAttachment: (url, name) => invoke<string>("save_attachment", { url, name }),
    setBadge: (count) => {
      void invoke("set_badge", { count });
    },
    setZoom: (scale) => invoke<void>("set_zoom", { scale }),
    listTasks: () => invoke<Task[]>("list_tasks"),
    addTask: (title, notes, due) => invoke<Task>("add_task", { title, notes: notes ?? null, due: due ?? null }),
    setTaskDone: (id, done) => invoke<Task>("set_task_done", { id, done }),
    deleteTask: (id) => invoke<void>("delete_task", { id }),
    notify: (title, body) => invoke<void>("notify", { title, body }),
    unfurl: (url) => invoke<LinkPreview | null>("unfurl", { url }),
    accountsList: () => invoke<Account[]>("accounts_list"),
    accountSwitch: (id) => invoke<Account>("account_switch", { id }),
    accountAdd: () => invoke<Account>("account_add"),
    accountRemove: (id) => invoke<void>("account_remove", { id }),
    availabilityGet: () => invoke<Availability>("availability_get"),
    availabilityMark: (state, ttlSeconds) => invoke<Availability>("availability_mark", { state, ttlSeconds: ttlSeconds ?? null }),
    availabilityStatus: (text, emoji, ttlSeconds) => invoke<Availability>("availability_status", { text: text ?? null, emoji: emoji ?? null, ttlSeconds: ttlSeconds ?? null }),
    calendarList: () => invoke<CalendarEntry[]>("calendar_list"),
    calendarEvents: (calendarId, o) => invoke<EventsPage>("calendar_events", { calendarId, timeMin: o.timeMin ?? null, timeMax: o.timeMax ?? null, syncToken: o.syncToken ?? null }),
    calendarCreateEvent: (calendarId, event) => invoke<CalEvent>("calendar_create_event", { calendarId, event }),
    calendarUpdateEvent: (calendarId, eventId, event) => invoke<CalEvent>("calendar_update_event", { calendarId, eventId, event }),
    calendarRespond: (calendarId, eventId, response) => invoke<CalEvent>("calendar_respond", { calendarId, eventId, response }),
    calendarDeleteEvent: (calendarId, eventId) => invoke<void>("calendar_delete_event", { calendarId, eventId }),
    notesLoad: () => invoke<string | null>("notes_load"),
    notesSave: (json) => invoke<void>("notes_save", { json }),
    createMeet: () => invoke<MeetSpace>("create_meet"),
    meetStatus: (code) => invoke<MeetStatus>("meet_status", { code }),
    postHuddle: (space, meetingUri, meetingCode, mentionAll) => invoke<Message>("post_huddle", { space, meetingUri, meetingCode, mentionAll: !!mentionAll }),
    onFileDrop: (cb) => {
      attachDragDrop();
      dropCbs.add(cb);
      return () => dropCbs.delete(cb);
    },
    onFileDrag: (cb) => {
      attachDragDrop();
      dragCbs.add(cb);
      return () => dragCbs.delete(cb);
    },
    relaySend: (msg: ClientMsg) => {
      void invoke("relay_send", { msg });
    },
    onMessage: (cb) => {
      attach();
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
  };
}
