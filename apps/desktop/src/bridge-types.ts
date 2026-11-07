// Contract between the web UI and whichever shell hosts it.
// Mirrors crates/prism-proto (Google Chat API v1 subset + relay protocol).
// Desktop: implemented over Tauri invoke/listen. Browser: fetch + WebSocket.

export interface User {
  name: string; // users/{id}
  displayName?: string;
  domainId?: string;
  type?: "HUMAN" | "BOT";
  isAnonymous?: boolean;
  avatarUrl?: string;
  email?: string;
}
export interface Space {
  name: string; // spaces/{id}
  spaceType?: "SPACE" | "GROUP_CHAT" | "DIRECT_MESSAGE";
  displayName?: string;
  spaceThreadingState?: "THREADED_MESSAGES" | "GROUPED_MESSAGES" | "UNTHREADED_MESSAGES";
  spaceDetails?: { description?: string; guidelines?: string };
  createTime?: string;
  lastActiveTime?: string;
  membershipCount?: { joinedDirectHumanUserCount?: number; joinedGroupCount?: number };
  spaceUri?: string;
}
export interface Membership {
  name: string; // spaces/{space}/members/{member}
  state?: "JOINED" | "INVITED" | "NOT_A_MEMBER";
  role?: "ROLE_MEMBER" | "ROLE_MANAGER";
  member?: User;
  createTime?: string;
}
export interface Thread { name?: string; threadKey?: string }
export interface Emoji { unicode?: string; customEmoji?: { name?: string; uid?: string; emojiName?: string; temporaryImageUri?: string } }
export interface EmojiReactionSummary { emoji?: Emoji; reactionCount?: number }
export interface Reaction { name?: string; user?: User; emoji: Emoji }
export interface Attachment {
  name?: string; contentName?: string; contentType?: string;
  attachmentDataRef?: { resourceName?: string; attachmentUploadToken?: string };
  /** Drive files: message lists carry only `name` + `source: "DRIVE_FILE"`; `Bridge.getAttachment` fills in the rest. */
  driveDataRef?: { driveFileId?: string };
  thumbnailUri?: string; downloadUri?: string; source?: "DRIVE_FILE" | "UPLOADED_CONTENT";
}
export interface MeetSpaceLinkData { meetingCode?: string; type?: "MEETING" | "HUDDLE" | string; huddleStatus?: "STARTED" | "ENDED" | "MISSED" | string }
export interface RichLinkMetadata { uri?: string; richLinkType?: "DRIVE_FILE" | "CHAT_SPACE" | "MEET_SPACE" | "CALENDAR_EVENT" | string; meetSpaceLinkData?: MeetSpaceLinkData }
export interface Annotation {
  type?: "USER_MENTION" | "SLASH_COMMAND" | "RICH_LINK";
  startIndex?: number; length?: number;
  userMention?: { user?: User; type?: "ADD" | "MENTION" };
  /** Google's huddle is a message with no text and one of these: a Meet space link with huddle type and status. */
  richLinkMetadata?: RichLinkMetadata;
}

/** Google Meet REST API: a meeting space and who is in its running conference. */
export interface MeetSpace { name: string; meetingUri: string; meetingCode: string }
export interface MeetParticipant { user?: string; displayName?: string; since?: string }
export interface MeetStatus { active: boolean; conferenceRecord?: string; participants: MeetParticipant[] }
export interface Message {
  name: string; // spaces/{space}/messages/{id}; optimistic ones end in client-{uuid}
  sender?: User;
  createTime?: string;
  lastUpdateTime?: string;
  deleteTime?: string; // set on a tombstone: Google keeps the row, drops the content
  deletionMetadata?: { deletionType?: string }; // CREATOR | SPACE_OWNER | ADMIN | APP_MESSAGE_EXPIRY | *_VIA_APP | SPACE_MEMBER
  text?: string; // markdown
  formattedText?: string;
  thread?: Thread;
  threadReply?: boolean;
  space?: Space;
  attachment?: Attachment[];
  emojiReactionSummaries?: EmojiReactionSummary[];
  annotations?: Annotation[];
  quotedMessageMetadata?: { name?: string; lastUpdateTime?: string };
  clientAssignedMessageId?: string;
  // UI-only, never sent: outbox state for optimistic messages
  _pending?: "sending" | "failed";
  _error?: string;
  // UI-only: which reaction unicodes are the current user's (the shell fills it from its cache when it can)
  _myReactions?: string[];
}
export interface Page<T> { items: T[]; nextPageToken?: string }

/** Open Graph summary of a page the user agreed to fetch. */
export interface LinkPreview { url: string; title?: string; description?: string; image?: string; site?: string }

/** Google Calendar v3, the slice the Calendar view needs. */
export interface CalendarEntry { id: string; summary: string; primary?: boolean; backgroundColor?: string; accessRole?: string }
export interface EventTime { dateTime?: string; date?: string; timeZone?: string }
export interface Attendee { email: string; displayName?: string; responseStatus?: "needsAction" | "declined" | "tentative" | "accepted" | string; self?: boolean; organizer?: boolean; optional?: boolean }
export interface CalEvent {
  id: string; status?: string; summary?: string; description?: string; location?: string; htmlLink?: string; hangoutLink?: string;
  start: EventTime; end: EventTime; attendees?: Attendee[]; organizer?: Attendee; conferenceData?: { entryPoints?: { entryPointType?: string; uri?: string; label?: string }[] };
  created?: string; updated?: string; recurringEventId?: string; colorId?: string; calendarId?: string; recurrence?: string[]; transparency?: string;
}
export interface EventsPage { items: CalEvent[]; nextSyncToken?: string; nextPageToken?: string }
export interface EventInput { summary: string; description?: string; location?: string; start: EventTime; end: EventTime; attendees: string[]; addMeet: boolean; recurrence?: string[] }

/** Google Tasks, default list only. `due` is an RFC 3339 date (the time part is ignored by Google). */
export interface Task { id: string; title: string; notes?: string; status: "needsAction" | "completed"; due?: string; updated?: string; webViewLink?: string }

export type PresenceState = "active" | "away" | "dnd" | "offline";
export interface CustomStatus { emoji?: string; text: string; expires_at?: string }
/** Google Chat's own availability for the signed-in user (users.availability). */
export interface Availability { name?: string; state?: "ACTIVE" | "IDLE" | "AWAY" | "DO_NOT_DISTURB" | "STATE_UNSPECIFIED" | string; customStatus?: { text?: string; emoji?: { unicode?: string }; expireTime?: string }; doNotDisturbMetadata?: { expirationTime?: string } }

export interface ChatEvent {
  event_type: string; // google.workspace.chat.message.v1.created | updated | deleted, reaction.v1.created | deleted, membership.v1.*, space.v1.updated
  space: string;
  event_time: string;
  message?: Message;
  reaction?: Reaction;
  membership?: Membership;
  space_resource?: Space;
}

// Relay WebSocket frames (snake_case, tagged by `type`)
export type ClientMsg =
  | { type: "hello"; token: string; device_id: string; client_version: string }
  | { type: "subscribe"; spaces: string[] }
  | { type: "typing"; space: string; thread?: string; active: boolean }
  | { type: "presence"; state: PresenceState; custom_status?: CustomStatus }
  | { type: "read"; space: string; thread?: string; message: string }
  | { type: "draft"; space: string; thread?: string; text: string }
  | { type: "huddle_join"; space: string }
  | { type: "huddle_leave"; space: string }
  | { type: "huddle_signal"; space: string; to: string; payload: unknown }
  | { type: "ping"; t: number };

export type ServerMsg =
  | { type: "welcome"; user: User; session_id: string; online: string[] }
  | { type: "typing"; space: string; thread?: string; user: string; active: boolean; ttl_ms: number }
  | { type: "presence"; user: string; state: PresenceState; custom_status?: CustomStatus; at: string }
  | { type: "receipt"; space: string; thread?: string; message: string; user: string; at: string }
  | { type: "draft"; space: string; thread?: string; text: string; device_id: string; at: string }
  | { type: "event"; event: ChatEvent }
  | { type: "huddle"; space: string; users: string[]; at: string }
  | { type: "huddle_signal"; space: string; from: string; payload: unknown }
  | { type: "pong"; t: number }
  | { type: "error"; code: string; message: string };

export type TYPING_TTL_MS = 5000;

export interface AuthStatus { signedIn: boolean; user?: User }

/** Shell-provided events (besides relay frames): socket state and outbox results. */
export type ShellEvent =
  | { type: "relay_state"; state: "connecting" | "open" | "closed" }
  | { type: "outbox"; clientId: string; status: "sent" | "failed"; message?: Message; error?: string }
  | { type: "account"; id: string }
  | { type: "huddle_window_closed" };

/** One Google account signed into this Confab, Slack-workspace style; the shell keeps a profile per account. */
export interface Account { id: string; email?: string; displayName?: string; photo?: string; signedIn: boolean; active: boolean; unread: number }

export interface Bridge {
  authStatus(): Promise<AuthStatus>;
  /** Opens the Google (or twin) consent page; resolves when signed in. */
  authStart(): Promise<AuthStatus>;
  signOut(): Promise<void>;

  listSpaces(): Promise<Space[]>;
  listMembers(space: string): Promise<Membership[]>;
  /** Newest first. `thread` restricts to a thread. */
  listMessages(space: string, opts?: { pageToken?: string; thread?: string; pageSize?: number }): Promise<Page<Message>>;
  /** Returns the optimistic message immediately (name ends in client-{clientId}); the shell emits an `outbox` event when Google accepts or rejects it. */
  sendMessage(
    space: string,
    text: string,
    opts?: { thread?: string; clientId?: string; attachments?: Attachment[] },
  ): Promise<Message>;
  editMessage(name: string, text: string): Promise<Message>;
  /** `force` deletes a thread parent together with its replies (Google refuses otherwise). */
  deleteMessage(name: string, force?: boolean): Promise<void>;
  /** Adds the reaction if the current user has not reacted with it, else removes it. */
  toggleReaction(messageName: string, unicode: string): Promise<void>;
  markRead(space: string, message: string, thread?: string): Promise<void>;
  /** Full-text search over the local cache (desktop) or over Google (browser). */
  search(query: string): Promise<Message[]>;
  /** Workspace directory search; resolves [] for a blank query. */
  searchPeople(query: string): Promise<User[]>;
  /** `user` is users/{id}; returns the existing DM space or a new one. */
  setupDm(user: string): Promise<Space>;
  /** `members` are users/{id}, at least two; the caller is added by the server. Returns the new group chat. */
  setupGroup(members: string[]): Promise<Space>;
  /** `members` are users/{id}; the caller is added by the server. */
  createSpace(displayName: string, members: string[]): Promise<Space>;
  /** Adds people (users/{id}) to a space; resolves the new memberships. */
  addMembers(space: string, users: string[]): Promise<Membership[]>;
  /** Deletes my own membership. */
  leaveSpace(space: string): Promise<void>;
  uploadAttachment(space: string, file: File): Promise<Attachment>;
  /** Desktop only: upload by filesystem path (from a Tauri drag-drop). */
  uploadAttachmentPath?(space: string, path: string): Promise<Attachment>;
  /**
   * Resolves the full resource for a bare `{name, source}` attachment: Google lists Drive attachments
   * with only those two fields, the rest (contentName, contentType, driveDataRef, downloadUri) comes from
   * `GET /v1/{attachment.name}`.
   */
  getAttachment(name: string): Promise<Attachment>;
  /** Opens the URL in the system browser (browser and mock shells use window.open); `account` steers a Google link to the matching Chrome profile on desktop. */
  openUrl(url: string, account?: string): Promise<void>;
  /** The real Meet in a small window, in the Chrome profile signed in as `account` when there is one; only the desktop shell has this, others open the browser. */
  openHuddleWindow?(url: string, title: string, account?: string): Promise<void>;
  closeHuddleWindow?(): Promise<void>;
  /** Brings Confab forward for an incoming huddle. */
  raiseWindow?(): Promise<void>;
  /** Desktop only: macOS dock badge; 0 clears it. */
  setBadge?(count: number): void;
  /** Desktop only: page zoom of the whole webview, 1 is 100%. */
  setZoom?(scale: number): Promise<void>;
  /** Google Tasks on the default list. */
  listTasks(): Promise<Task[]>;
  addTask(title: string, notes?: string, due?: string): Promise<Task>;
  setTaskDone(id: string, done: boolean): Promise<Task>;
  deleteTask(id: string): Promise<void>;
  /** Desktop and mock: fetch a page's Open Graph tags after the user consented; null when the page says nothing useful. */
  unfurl?(url: string): Promise<LinkPreview | null>;
  /** Accounts side by side: list, switch (the webview reboots its workspace), add (a fresh profile, signed out), remove. */
  accountsList(): Promise<Account[]>;
  accountSwitch(id: string): Promise<Account>;
  accountAdd(): Promise<Account>;
  accountRemove(id: string): Promise<void>;
  /** Google Chat presence for the signed-in user: read it, mark active, away or do-not-disturb, set or clear the custom status. */
  availabilityGet(): Promise<Availability>;
  availabilityMark(state: "active" | "away" | "dnd", ttlSeconds?: number): Promise<Availability>;
  availabilityStatus(text?: string, emoji?: string, ttlSeconds?: number): Promise<Availability>;
  /** Google Calendar: calendars, events in a window or since a sync token, create, respond, delete. */
  calendarList(): Promise<CalendarEntry[]>;
  calendarEvents(calendarId: string, opts: { timeMin?: string; timeMax?: string; syncToken?: string }): Promise<EventsPage>;
  calendarCreateEvent(calendarId: string, event: EventInput): Promise<CalEvent>;
  calendarUpdateEvent(calendarId: string, eventId: string, event: EventInput): Promise<CalEvent>;
  calendarRespond(calendarId: string, eventId: string, response: "accepted" | "declined" | "tentative"): Promise<CalEvent>;
  calendarDeleteEvent(calendarId: string, eventId: string): Promise<void>;
  /** Confab's notes, one JSON document in the user's Drive app data (Keep has no API for a signed-in user). */
  notesLoad(): Promise<string | null>;
  notesSave(json: string): Promise<void>;
  /** Google Meet: a new meeting space, who is in a running one, and the huddle message Google Chat understands. */
  createMeet(): Promise<MeetSpace>;
  meetStatus(code: string): Promise<MeetStatus>;
  postHuddle(space: string, meetingUri: string, meetingCode: string, mentionAll?: boolean): Promise<Message>;
  /** A system notification (desktop: the notification plugin; browsers: the Notification API once allowed). */
  notify?(title: string, body: string): Promise<void>;
  /** Desktop only: saves an attachment URL (prism-media scheme or chat API) to Downloads and returns the path. */
  saveAttachment?(url: string, name: string): Promise<string>;
  /** Desktop only: OS file drops onto the window, as paths. */
  onFileDrop?(cb: (paths: string[]) => void): () => void;
  /** Desktop only: true while OS files are dragged over the window, false on leave or drop. */
  onFileDrag?(cb: (over: boolean) => void): () => void;

  relaySend(msg: ClientMsg): void;
  onMessage(cb: (m: ServerMsg | ShellEvent) => void): () => void;
}
