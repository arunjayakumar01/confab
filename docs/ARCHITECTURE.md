# Confab

A desktop client for Google Chat (Google Workspace Business or Enterprise).
Tauri 2 desktop shell with a glass UI, a Rust relay for the things Google's API does not
carry (typing, presence, read receipts, draft sync), and a local **Google twin** that
speaks the same API so the whole product runs on one laptop with no Google account.

```
crates/confab-proto        shared types: Chat API v1 subset, relay protocol, Pub/Sub envelope
services/google-twin      local stand-in for Google: OAuth + Chat API subset + event push  (:8081)
services/relay            WebSocket relay + Pub/Sub push endpoint + Workspace Events manager (:8080)
apps/desktop              Tauri 2 app: src-tauri = Rust core, src = React web view (:1420 in dev)
```

## Contract (read this before touching any package)

### Endpoints and modes
`confab_proto::Endpoints::from_env()` is the single source of configuration, in every process:

| env | twin default | real Google |
| --- | --- | --- |
| `PRISM_TWIN_BASE` | `http://127.0.0.1:8081` | unused |
| `PRISM_CHAT_API` | twin | `https://chat.googleapis.com` |
| `PRISM_OAUTH_AUTH` | twin `/o/oauth2/v2/auth` | `https://accounts.google.com/o/oauth2/v2/auth` |
| `PRISM_OAUTH_TOKEN` | twin `/token` | `https://oauth2.googleapis.com/token` |
| `PRISM_USERINFO` | twin `/oauth2/v3/userinfo` | `https://openidconnect.googleapis.com/v1/userinfo` |
| `PRISM_RELAY_WS` | `ws://127.0.0.1:8080/ws` | `wss://relay.<your domain>/ws` |
| `PRISM_PEOPLE_API` | twin | `https://people.googleapis.com` |
| `PRISM_TASKS_API` | twin | `https://tasks.googleapis.com` |
| `PRISM_MEET_API` | twin | `https://meet.googleapis.com` |
| `PRISM_CALENDAR_API` | twin `/calendar/v3` | `https://www.googleapis.com/calendar/v3` |
| `PRISM_DRIVE_API` | twin `/drive/v3` | `https://www.googleapis.com/drive/v3` (uploads go to the same host under `/upload/drive/v3`) |
| `PRISM_CLIENT_ID` | `prism-dev` | the internal OAuth client id |

The web view reads the same names with a `VITE_` prefix in browser mode.
The relay additionally reads `PRISM_PUSH_SECRET` (twin mode shared secret, default `dev`),
`PRISM_PUSH_AUDIENCE` + `PRISM_PUSH_SERVICE_ACCOUNT` (real mode, Pub/Sub OIDC), `PRISM_PUBSUB_TOPIC`
(`projects/<p>/topics/<t>`, real mode) and `PRISM_DB` (SQLite path, default `relay.db`).
Real Google mode is detected by `PRISM_CHAT_API` containing `googleapis.com`.

### Google Chat API v1 subset (what the client calls, what the twin serves)
All paths under `PRISM_CHAT_API`. Bearer token on every call. JSON camelCase as in `confab_proto::chat`.

| call | method and path | notes |
| --- | --- | --- |
| list spaces | `GET /v1/spaces?pageSize=100&pageToken=` | `ListSpacesResponse` |
| get space | `GET /v1/spaces/{s}` | |
| list members | `GET /v1/spaces/{s}/members?pageSize=100` | `ListMembershipsResponse` |
| get my membership | `GET /v1/spaces/{s}/members/users/{id}` | 403/404 when not a member; the relay uses this to authorise subscriptions |
| list messages | `GET /v1/spaces/{s}/messages?pageSize=100&orderBy=createTime%20desc&showDeleted=true&pageToken=&filter=` | filter `thread.name = "spaces/{s}/threads/{t}"` for one thread; replies carry `threadReply: true` and appear in the space list too |
| create message | `POST /v1/spaces/{s}/messages?messageId=client-{uuid}&messageReplyOption=REPLY_MESSAGE_FALLBACK_TO_NEW_THREAD` body `{text, thread?: {name}}` | returns the `Message`; `name` is server-assigned, `clientAssignedMessageId` echoes the id. Same `messageId` twice = 409 |
| update message | `PATCH /v1/spaces/{s}/messages/{m}?updateMask=text` body `{text}` | |
| delete message | `DELETE /v1/spaces/{s}/messages/{m}` | Google then lists it as a tombstone (`deleteTime`, `deletionMetadata.deletionType`, no content) when `showDeleted=true`; the twin answers 404 and drops it |
| list reactions | `GET /v1/spaces/{s}/messages/{m}/reactions?filter=` | filter `user.name = "users/{id}"` |
| create reaction | `POST /v1/spaces/{s}/messages/{m}/reactions` body `{emoji: {unicode}}` | |
| delete reaction | `DELETE /v1/spaces/{s}/messages/{m}/reactions/{r}` | |
| read state | `GET\|PATCH /v1/users/me/spaces/{s}/spaceReadState?updateMask=lastReadTime` body `{lastReadTime}` | |
| search | `GET /v1/spaces/-/messages:search?query=&pageSize=50` | twin only today; desktop searches its local FTS cache |
| directory search | `GET {people_api}/v1/people:searchDirectoryPeople?query=&readMask=names,emailAddresses,photos&sources=DIRECTORY_SOURCE_TYPE_DOMAIN_PROFILE` | People API; `people/{id}` is Chat's `users/{id}`; scope `directory.readonly` |
| new DM / new space | `POST /v1/spaces:setup` | `{space:{spaceType:"DIRECT_MESSAGE"}, memberships:[{member:{name:"users/{id}",type:"HUMAN"}}]}` returns the existing DM or a new one; `spaceType:"SPACE"` + `displayName` creates a space (409 ALREADY_EXISTS on a taken name); scope `chat.spaces.create` |
| new group chat | `POST /v1/spaces:setup` | `{space:{spaceType:"GROUP_CHAT"}, memberships:[two or more]}` creates a group chat with the caller and every listed user as `ROLE_MEMBER`, no `displayName`; same scope |
| upload | `POST /upload/v1/spaces/{s}/attachments:upload?uploadType=multipart` multipart: `metadata` JSON `{filename}` + file part | `UploadAttachmentResponse`; then create message with `attachment: [{attachmentDataRef}]` |
| get attachment | `GET /v1/{attachment name}` (`spaces/{s}/messages/{m}/attachments/{a}`) | the full `Attachment`; a message list carries a Drive attachment with only `name` and `source: "DRIVE_FILE"`, `driveDataRef.driveFileId` comes from this call |
| download | `GET /v1/media/{resourceName}?alt=media` | |
| userinfo | `GET <PRISM_USERINFO>` | `UserInfo`; `sub` = Chat user id, user name = `users/{sub}` |
| list tasks | `GET {tasks_api}/tasks/v1/lists/@default/tasks?showCompleted=true&showHidden=false&maxResults=100` | Google Tasks, the default list; `{items: Task[]}` (`items` absent when empty); scope `tasks` |
| add task | `POST {tasks_api}/tasks/v1/lists/@default/tasks` body `{title, notes?, due?}` | the `Task`; the twin answers 201 with ids like `task-3` |
| task done / undone | `PATCH {tasks_api}/tasks/v1/lists/@default/tasks/{id}` body `{status: "completed"}` or `{status: "needsAction", completed: null}` | the `Task`; the twin also merges `title`, `notes`, `due` |
| delete task | `DELETE {tasks_api}/tasks/v1/lists/@default/tasks/{id}` | 204, no body; 404 after |
| create meet | `POST {meet_api}/v2/spaces` body `{config: {accessType: "TRUSTED"}}` | Google Meet REST API v2; `{name, meetingUri, meetingCode, config}`; scope `meetings.space.created`; the twin names them `spaces/meet-{n}` with codes like `abc-defg-hij` |
| get meet | `GET {meet_api}/v2/spaces/{code}` (`abc-defg-hij` or `spaces/{id}`) | the space, plus `activeConference: {conferenceRecord}` while a call is running (the twin: fifteen minutes after creation); 404 for an unknown code; scope `meetings.space.readonly` |
| meet participants | `GET {meet_api}/v2/{conferenceRecord}/participants?pageSize=100` | `{participants: [{signedinUser: {user, displayName}} \| {anonymousUser: {displayName}} \| {phoneUser: {displayName}}, earliestStartTime, latestEndTime?]}`; no `latestEndTime` means still in the call. The twin lists the creator, Priya (joins five seconds in) and Rahul (already left) |
| huddle message | `POST /v1/spaces/{s}/messages?messageId=client-{uuid}` body `{text: "", annotations: [{type: "RICH_LINK", startIndex: 0, richLinkMetadata: {uri, richLinkType: "MEET_SPACE", meetSpaceLinkData: {meetingCode, type: "HUDDLE", huddleStatus: "STARTED"}}}]}` | how Google itself posts a huddle (and later updates it with `huddleStatus: ENDED` or `MISSED`); the twin keeps the annotation. If Google refuses the annotation (400), the shell posts `Started a huddle` plus the link instead |
| availability | `GET /v1/users/me/availability` | `Availability` `{name, state: ACTIVE \| IDLE \| AWAY \| DO_NOT_DISTURB \| STATE_UNSPECIFIED, customStatus?: {text, emoji: {unicode}, expireTime?}, doNotDisturbMetadata?: {expirationTime}}` (`confab_proto::availability`); scope `chat.users.availability`. The twin keeps one record per caller, `ACTIVE` by default, and accepts `me` or the caller's own id in the path |
| mark availability | `POST /v1/users/me/availability:markAsActive` body `{ttl?: "1800s"}` (Google defaults to thirty minutes), `:markAsAway` empty body, `:markAsDoNotDisturb` body `{ttl: "3600s"}` (required, at most a year) | each answers the `Availability`; the twin sets `doNotDisturbMetadata.expirationTime` to now plus `ttl`, reports `IDLE` once an active window lapses and `ACTIVE` once do-not-disturb does |
| custom status | `PATCH /v1/users/me/availability?updateMask=customStatus` body `{customStatus: {text (64 chars max), emoji: {unicode}, ttl?: "3600s"}}`; `{customStatus: {}}` with the same mask clears it (the shell falls back to an empty body with the mask if Google answers 400) | the `Availability`; the twin answers 400 for text over 64 characters, a missing emoji, or a missing mask, and sets `expireTime` when a `ttl` was given |
| calendar list | `GET {calendar_api}/users/me/calendarList?minAccessRole=reader&maxResults=250&pageToken=` | Google Calendar v3; `{items: [{id, summary, primary?, backgroundColor, accessRole}]}` (`confab_proto::calendar::CalendarEntry`); scope `calendar.readonly`. The twin lists `primary` (the caller's name, owner) and `team@example.test` (Team, reader) |
| list events | `GET {calendar_api}/calendars/{id}/events?singleEvents=true&maxResults=250&showDeleted=true` plus `timeMin=&timeMax=&orderBy=startTime` (a window: events whose end is after `timeMin` and start before `timeMax`) or `syncToken=` (only what changed since, cancelled ones as `{id, status: "cancelled"}`, no bounds or ordering) | `EventsPage` `{items: CalEvent[], nextSyncToken, nextPageToken?}`; the shell follows pages and stamps `calendarId` on every item; a 410 means the token expired and the shell's error starts with `sync_expired`. The twin's sync token is a change counter plus one, kept for the last 100 changes; an unknown, stale or future token is a 410 with `status: "fullSyncRequired"`; scope `calendar.events` |
| create event | `POST {calendar_api}/calendars/{id}/events?sendUpdates=all[&conferenceDataVersion=1]` body `{summary, description?, location?, start: {dateTime \| date}, end, attendees: [{email}], recurrence?: ["RRULE:..."], conferenceData?: {createRequest: {requestId, conferenceSolutionKey: {type: "hangoutsMeet"}}}}` | the `CalEvent` (`{id, status, summary, description?, location?, htmlLink, hangoutLink?, start, end, attendees: [{email, displayName?, responseStatus, self?, organizer?, optional?}], organizer, conferenceData?: {entryPoints: [{entryPointType, uri, label?}]}, created, updated, recurringEventId?, recurrence?, transparency?, colorId?}`); the twin names them `ev-{n}`, adds the caller as organizer (`accepted`), the others `needsAction`, and a `createRequest` with `conferenceDataVersion=1` mints a `hangoutLink` plus a video entry point. The team calendar answers 403 for writes |
| update event | `PATCH {calendar_api}/calendars/{id}/events/{eventId}?sendUpdates=all[&conferenceDataVersion=1]` body: any of the create fields | the merged `CalEvent`; `attendees` replaces the list; a `createRequest` adds a Meet to an event without one and leaves an existing link alone |
| respond to event | the same `PATCH` with body `{attendees: [the whole list, the caller's entry with responseStatus: accepted \| declined \| tentative]}` | the `CalEvent`; the shell fetches the event, flips the attendee marked `self`, and patches the list back |
| delete event | `DELETE {calendar_api}/calendars/{id}/events/{eventId}?sendUpdates=all` | 204, no body; the twin keeps the event as `cancelled` for syncs and answers 410 to a second delete |
| notes file | `GET {drive_api}/files?spaces=appDataFolder&q=name%3D%27prism-notes.json%27&fields=files(id,name,modifiedTime)` then `GET {drive_api}/files/{id}?alt=media` | Drive app-data folder; `{files: [{id, name, modifiedTime}]}` then the JSON text; scope `drive.appdata`. The twin keeps app data per caller with ids `file-{n}` |
| drive list | `GET {drive_api}/files?q=&orderBy=&fields=nextPageToken,files(...)&pageSize=100&supportsAllDrives=true&includeItemsFromAllDrives=true&corpora=allDrives` (or `corpora=drive&driveId=` inside a shared drive) | `{files, nextPageToken?}`; scope `drive`. The web view builds `q` (`lib/drive.ts`): `'id' in parents`, `sharedWithMe`, `starred`, `trashed = bool`, `fullText contains '...'`, `mimeType != '...'`, joined by `and`; `orderBy` is empty for `fullText` searches, which Google refuses to sort. The twin accepts that grammar and matches `fullText` against names |
| drive write | `POST {drive_api}/files` (folder metadata), `POST {upload}/files?uploadType=multipart` with `parents`, `PATCH {drive_api}/files/{id}[?addParents=&removeParents=]` body any of `{name, starred, trashed}`, `GET {drive_api}/files/{id}/export?mimeType=application/pdf` for Docs Editors files, `GET {drive_api}/drives` | the file with the same fields; scope `drive` |
| save notes | `PATCH {upload}/files/{id}?uploadType=media` (content-type `application/json`, the text) when the file exists, else `POST {upload}/files?uploadType=multipart` with a `multipart/related` body: `{"name": "prism-notes.json", "parents": ["appDataFolder"]}` as `application/json`, then the text as `application/json`; `{upload}` is `{drive_api}` with `/upload/drive/v3` in place of `/drive/v3` | `{id, name, mimeType, modifiedTime}` |

OAuth: authorization code + PKCE (S256), loopback redirect `http://127.0.0.1:{port}/callback`,
scopes = `confab_proto::SCOPES`, `access_type=offline&prompt=consent` to get a refresh token.
`DRIVE_SCOPE` (`drive`) is not in `SCOPES`: the Drive view asks for it on its own with `include_granted_scopes=true`
and `login_hint`, so adding it never signs anyone out. A different account picked in the browser is refused. The twin
remembers each token's scopes, keeps them across refreshes, and answers 403 `PERMISSION_DENIED` to a Drive call
without `drive`.
Token endpoint takes form fields `grant_type`, `code`, `code_verifier`, `redirect_uri`, `client_id`
(and `refresh_token` for `grant_type=refresh_token`).
Errors are Google-shaped: `{"error": {"code", "message", "status"}}` with 401/403/404/409/429.

### Workspace Events
Events reach the relay at `POST /events/push` in the Cloud Pub/Sub push shape (`confab_proto::events::PubSubPush`):
attributes `ce-type`, `ce-subject` (`//chat.googleapis.com/spaces/{s}`), `ce-time`; `data` = base64 JSON `EventData`.
Twin mode: header `X-Prism-Push-Secret` or a `?secret=` query parameter on the push URL (a Pub/Sub push subscription cannot set headers). Real mode: `Authorization: Bearer <Google OIDC JWT>` verified against
`https://www.googleapis.com/oauth2/v3/certs`, audience `PRISM_PUSH_AUDIENCE`, email `PRISM_PUSH_SERVICE_ACCOUNT`.
Real mode subscriptions: `POST https://workspaceevents.googleapis.com/v1/subscriptions`
`{targetResource: "//chat.googleapis.com/spaces/-", eventTypes: [...], notificationEndpoint: {pubsubTopic},
payloadOptions: {includeResource: true}}` with the user's token, renewed with
`PATCH /v1/{name}?updateMask=ttl` body `{ttl: "0s"}` before `expireTime`. The twin pushes every event itself, so
in twin mode the subscription manager does nothing. The twin posts straight to the relay by default; with
`PRISM_PUBSUB_EMULATOR_HOST` (e.g. `localhost:4588`) and `PRISM_PUBSUB_TOPIC` set it publishes to that Pub/Sub
emulator instead, and a push subscription delivers to the relay, which is the shape real GCP has.

### Relay WebSocket protocol
`confab_proto::relay::{ClientMsg, ServerMsg}`, JSON text frames tagged by `type` (snake_case).
First frame must be `hello`; the relay calls userinfo with the token and answers `welcome` or `error` + close.
`subscribe` replaces the session's space set; each space is authorised with "get my membership" (cached 10 min).
`typing` fans out to every other session subscribed to that space with `ttl_ms = 5000`.
`presence` fans out to every session that shares a subscribed space; `welcome.online` lists connected users.
`read` fans out as `receipt` to the space's subscribers and is stored (last receipt per user per space/thread).
`draft` fans out only to the same user's other devices and is stored. `ping` → `pong`.
Google events arrive as `event` to subscribers of `event.space`, deduplicated by `message.name + lastUpdateTime`
(or reaction/membership name) for 60 s.

### Web view ↔ shell bridge
`apps/desktop/src/bridge-types.ts` is the TypeScript contract. Two implementations in `apps/desktop/src/bridge/`:
`desktop.ts` (Tauri `invoke` + `listen`) and `browser.ts` (fetch + WebSocket straight to the endpoints).
Pick with `"__TAURI_INTERNALS__" in window`.

Tauri commands (JS sends camelCase keys, Rust params are snake_case; Tauri converts):

| command | args | returns |
| --- | --- | --- |
| `auth_status` | | `AuthStatus` |
| `auth_start` | | `AuthStatus` (runs the whole PKCE flow, opens the system browser) |
| `sign_out` | | |
| `list_spaces` | | `Space[]` |
| `list_members` | `space` | `Membership[]` |
| `search_people` | `query` | `User[]` from the directory; `[]` for a blank query |
| `setup_dm` | `user` (`users/{id}`) | `Space` (existing or new DM) |
| `setup_group` | `members: string[]` (`users/{id}`, two or more) | `Space` with `spaceType: GROUP_CHAT` (`POST /v1/spaces:setup`), caller included |
| `create_space` | `displayName`, `members: string[]` | `Space` |
| `list_messages` | `space, pageToken?, thread?, pageSize?` | `{items: Message[], nextPageToken?}` newest first |
| `send_message` | `space, text, thread?, clientId?` | optimistic `Message` named `spaces/{s}/messages/client-{id}`; later a `shell` event `{type:"outbox", clientId, status, message?}` |
| `edit_message` | `name, text` | `Message` |
| `delete_message` | `name`, `force` (optional, also deletes the thread's replies) | deletes for everyone; Google refuses a parent with replies unless `force` |
| `toggle_reaction` | `messageName, unicode` | |
| `mark_read` | `space, message, thread?` | |
| `search` | `query` | `Message[]` |
| `upload_attachment` | `space, path` | `Attachment` (desktop passes a file path; browser passes a `File`) |
| `upload_attachment_bytes` | raw IPC body = the file bytes; headers `x-prism-space` (space name), `x-prism-name` (filename, base64 standard of UTF-8), `x-prism-mime?` (default `application/octet-stream`) | `Attachment`, same shape as `upload_attachment`; JS: `invoke("upload_attachment_bytes", new Uint8Array(bytes), {headers})` |
| `get_attachment` | `name` (`spaces/{s}/messages/{m}/attachments/{a}`) | `Attachment` from `GET /v1/{attachment name}`, URIs proxied like `list_messages` |
| `open_url` | `url`, `account` (optional email) | opens http(s) links in the system browser; with `account`, a Google link goes to the Chrome profile signed in as that account when there is one (Chrome's local profile list, read on this machine only) |
| `huddle_window_open` | `url`, `title` | the real Meet in an always-on-top mini window for the active account; only `https://meet.google.com` links; the window has no access to these commands |
| `huddle_window_close` | | closes it without a `huddle_window_closed` event |
| `set_badge` | `count` (`0` clears) | shell only: macOS dock badge; a no-op where there is none |
| `add_members` | `space`, `users` (`users/{id}`) | adds each person (`spaces.members.create`); stops at the first failure and names the user |
| `leave_space` | `space` | deletes the caller's own membership |
| `unfurl` | `url` | link preview after consent: Open Graph title, description, image and site; refuses local hosts, 512 KB and 6 s caps |
| `set_zoom` | `scale` (`1.0` is 100%, clamped 0.5 to 2) | shell only: page zoom of the webview, driven by ⌘= ⌘- ⌘0 |
| `save_attachment` | `url` (a `prism-media` scheme URL or the upstream itself), `name` (file name) | shell only: fetches the attachment with the signed-in token (same origin allow-list as the media proxy), writes it to the Downloads folder (app data dir as fallback) as `name`, `name (2).ext`, ... and reveals it in the file manager; returns the absolute path |
| `list_tasks` | | `Task[]` from the signed-in user's default Google Tasks list, completed ones included; `Task` is `{id, title, notes?, status: "needsAction" \| "completed", due?, updated?, webViewLink?}` (`confab_proto::tasks::Task`) |
| `add_task` | `title`, `notes?`, `due?` (RFC 3339; Google keeps the date) | the created `Task`; a blank `title` is an error |
| `set_task_done` | `id`, `done: boolean` | the updated `Task` (`completed` on `true`, back to `needsAction` on `false`) |
| `delete_task` | `id` | |
| `create_meet` | | `MeetSpace` `{name, meetingUri, meetingCode}` (`confab_proto::meet::MeetSpace`): a fresh Google Meet space with `accessType: TRUSTED` |
| `meet_status` | `code` (`abc-defg-hij` or `spaces/{id}`) | `MeetStatus` `{active, conferenceRecord?, participants: [{user?, displayName?, since?}]}`: whether the space has a call running and who is still in it; an unknown code is an error |
| `post_huddle` | `space`, `meetingUri`, `meetingCode` | the huddle `Message` as Google stored it: no text and one `RICH_LINK` annotation (`richLinkMetadata.meetSpaceLinkData` with `type: HUDDLE`, `huddleStatus: STARTED`); when Google refuses the annotation, a plain `Started a huddle` message with the link. Cached like any send |
| `availability_get` | | `Availability` `{name, state, customStatus?: {text, emoji: {unicode}, expireTime?}, doNotDisturbMetadata?: {expirationTime}}` (`confab_proto::availability::Availability`) for the signed-in user |
| `availability_mark` | `state` (`active` \| `away` \| `dnd`), `ttlSeconds?` | the `Availability`: `active` sends `markAsActive` (with `ttl` when given, Google's default is thirty minutes), `away` sends `markAsAway`, `dnd` sends `markAsDoNotDisturb` with `ttl` (an hour when omitted) |
| `availability_status` | `text?`, `emoji?`, `ttlSeconds?` | the `Availability`: sets the custom status (`text` up to 64 characters, `emoji` defaults to a speech balloon, `ttlSeconds` optional); a blank or missing `text` clears it |
| `calendar_list` | | `CalendarEntry[]` `{id, summary, primary, backgroundColor?, accessRole?}` (`confab_proto::calendar::CalendarEntry`): every calendar the signed-in user can read |
| `calendar_events` | `calendarId`, `timeMin?`, `timeMax?`, `syncToken?` (RFC 3339 bounds) | `EventsPage` `{items: CalEvent[], nextSyncToken?}`: single instances of the window ordered by start, or with `syncToken` only what changed since (cancelled ones with `status: "cancelled"`); every item carries `calendarId`. `CalEvent` is `{id, status?, summary?, description?, location?, htmlLink?, hangoutLink?, start: {dateTime?, date?, timeZone?}, end, attendees: [{email, displayName?, responseStatus?, self, organizer, optional}], organizer?, conferenceData?: {entryPoints: [{entryPointType?, uri?, label?}]}, created?, updated?, recurringEventId?, recurrence?: string[], transparency?, colorId?, calendarId?}`. An expired token is an error whose message starts with `sync_expired`: refetch the window without a token |
| `calendar_create_event` | `calendarId`, `event: EventInput` | the created `CalEvent`. `EventInput` is `{summary, description?, location?, start: {dateTime?, date?, timeZone?}, end, attendees: string[] (emails), recurrence?: string[] (RRULE lines), addMeet: boolean}`; all-day events use `date` on both ends; `addMeet` attaches a Google Meet; invitations go out (`sendUpdates=all`) |
| `calendar_update_event` | `calendarId`, `eventId`, `event: EventInput` | the updated `CalEvent`: the input's fields replace the event's (attendees as a whole list, `recurrence` when given); `addMeet` on an event without a conference attaches one |
| `calendar_respond` | `calendarId`, `eventId`, `response` (`accepted` \| `declined` \| `tentative`) | the `CalEvent` with the signed-in attendee's `responseStatus` changed; an error when the user is not an attendee |
| `calendar_delete_event` | `calendarId`, `eventId` | deletes for everyone (`sendUpdates=all`) |
| `notes_load` | | the notes JSON as text (`string \| null`): `prism-notes.json` from the Drive app-data folder, `null` before the first save |
| `notes_save` | `json` (the text to store) | creates or replaces `prism-notes.json` in the app-data folder |
| `drive_status` | | `bool`: the stored sign-in includes the `drive` scope |
| `drive_connect` | | `bool`: runs the incremental consent for `drive` in the browser |
| `drive_list` | `q`, `orderBy`, `driveId?`, `pageToken?` | Drive's `{files, nextPageToken?}` |
| `drive_drives` | | Drive's `{drives: [{id, name}]}` |
| `drive_create_folder` | `name`, `parent` | the new folder |
| `drive_upload` | `path`, `parent` | the uploaded file (a dropped path) |
| `drive_upload_bytes` | raw IPC body = the bytes; headers `x-prism-parent`, `x-prism-name` (base64 of UTF-8), `x-prism-mime?` | the uploaded file |
| `drive_update` | `id`, `patch` (`name`, `starred`, `trashed`), `addParents?`, `removeParents?` | the updated file |
| `drive_content_url` | `id`, `mime` | a `prism-media` URL for the bytes, or a PDF export of a Docs Editors file; the Drive API origin is allowed on that scheme |
| `notify` | `title`, `body` | shell only: a native notification through the notification plugin. A `cargo tauri dev` build is not an app bundle, so macOS shows these as from Terminal and asks permission once. |
| `relay_send` | `msg: ClientMsg` | |
| `accounts_list` | | `Account[]` `{id, email?, displayName?, photo?, signedIn, active, unread}`: every profile under the app data dir, the active one first, then by id. `id` is the profile directory name; `unread` counts messages that arrived while the account was not active |
| `account_switch` | `id` | the `Account` made active (its unread count cleared); a `shell` event `{type: "account", id}` follows so the web view reboots its workspace, then that account's `relay_state` |
| `account_add` | | a fresh signed-out `Account` named `acct-<n>` (the first free n), made active like `account_switch`; sign it in with `auth_start` |
| `account_remove` | `id` | signs the account out, drops it and deletes its profile directory; refused for the active account |

Shell → web view events: `relay` (payload `ServerMsg`) and `shell` (payload `ShellEvent`, one of
`relay_state`, `outbox`, `account`). Only the active account's events reach the web view: a
`message.v1.created` for another account bumps its `unread` instead and, when someone else sent it,
raises one system notification titled `<displayName or email>: new message`, at most one per account per minute.

### Accounts
Every directory under the app data dir (`~/Library/Application Support/<bundle id>` on macOS) that holds
an `auth.json` or an `account.json` is one Google account: its own token store, `cache.db`, `device_id` and
relay socket. `PRISM_PROFILE=<name>` picks the one active at launch (`default` when unset); the others load
right after and keep their sockets open. `account.json` (`{email, displayName, photo}`) is written on sign-in
and on `auth_status`, so `accounts_list` needs no network call. The media proxy, `save_attachment` and every
other command act on the active account.

### Running two desktop instances
`PRISM_PROFILE=<name>` namespaces the data dir and token store so two signed-in users can run side by side.
The twin's sign-in page accepts `&twin_user=users/1002` to skip the click, and `POST /twin/token?user=users/1002`
mints a token for scripts.

### In the app
Keyboard: `⌘K` jump to a space or person, `⌘N` new message (one person opens a DM, two or more open a group chat),
`⌘F` search, `⌘.` thread panel, `⌘[` / `⌘]` back and forward through spaces, `⌥↑` / `⌥↓` previous and next space
(`⇧` for unread only), `⌘⇧A` Unreads, `⌘⇧S` Saved, `⌘=` / `⌘-` / `⌘0` zoom (desktop, `set_zoom`, remembered),
`⌘/` the shortcut list, `↑` in an empty composer edits your last message, `Esc` closes whatever is on top.

Message blocks: a sender run (same sender, within five minutes, same day) is one block: a soft rounded card with the
avatar, name and time once, then tight follow-up lines that show their time in the gutter on hover. Each virtual row
draws its share of the card (`msg--solo/first/mid/last` from `positions()` in MessageList.tsx), so the join needs
integral row heights (22 px line boxes). One to three emoji alone render large. Under a message, the small avatars
are the people whose read position is that message; click them for names and times. These come from the relay's
receipts, which other Confab clients send, because Google does not expose anyone else's read state to user auth, so
in a workspace where only you run Confab the strip stays empty. Delete opens Google Chat's dialog (for everyone, with
the thread's reply count); your own messages always, anyone's when you manage the space.

Messages: `lib/markdown.ts` renders Chat's markup plus numbered lists (a run keeps its count, `1.` then `2.`, a
stray number is not a list), `[label](url)` links, quotes and fenced code (with a copy button and language label).
Links open in the system browser through `open_url`. Anything longer than about sixteen lines folds at 300 px with a
mask fade and a Show more pill; the virtual row re-measures when it opens. Right-click a message (or its ⋯) for the
context menu: quick reactions, reply, save, mark unread from here, copy, edit, delete. A deleted message stays in
place as Google Chat shows it, "Message deleted" (or "by a space manager", or "expired") with the sender and time and
nothing else; the local delete and the relay's deleted event both leave that tombstone. The hover toolbar carries the
three most-used reactions (`recentEmoji`, persisted) and a bookmark. Inline images open in a lightbox with Open and
Save.

Views: Unreads (`⌘⇧A`) groups unread messages per space with Mark as read per group and for all; Saved (`⌘⇧S`) lists
bookmarked messages with Jump to message, which opens the space, loads older pages if needed, scrolls the row into
view and flashes it. Opening a space with unread messages draws a red New line above them (`ui.unreadAtOpen`, frozen
per mount). Mark unread from here sets the count and holds it (`holdRead`) until the space is opened again from
elsewhere. Right-click a sidebar row to star, mute, mark as read, copy its link or open it in Google Chat. Muted
spaces show a crossed bell instead of a badge and never count toward the dock badge, the rail dots or the chime. The
sidebar edge drags to resize (double-click resets). A row shows a pencil while it holds an unsent draft.

Files: clicking an attachment card opens the in-app viewer (`Viewer` in Overlays.tsx, bodies in Viewers.tsx). The
bytes come through the `prism-media` proxy (which echoes the webview's Origin so `fetch` may read them), or a data:
URL in the mock, capped at 60 MB. Viewers: images; PDFs through PDF.js on the main thread (the worker module is handed
to PDF.js as `globalThis.pdfjsWorker`, because in the Tauri webview the Web Worker started but never answered and
documents stayed on "Opening"; a 20 second cap turns a stuck parse into an error), pages painted on canvases as
they scroll into view, with a page counter and zoom; Word through mammoth on a paper sheet (scripts and script links stripped); Excel,
CSV and ODS through SheetJS as a grid with sheet tabs and column letters; zip archives as a tree whose entries open
in place through fflate with a Back button to the archive; text and code with line numbers; video and audio. The
bar shows the kind, name, size, Open for http links and Save. Presentations have no renderer yet and save instead;
Drive files always open in the browser because they need Google's own session. The libraries load on first use.

The space header: an emblem in the space's own colour (a stable hue from its name), the name as a menu, the
description with member and online counts, a facepile into the members panel, then Meet, Files, Tasks and Pinned
and the usual tools; a spectrum line sweeps along the bottom each time the space changes.

Huddles, the way Google Chat does them: Google represents a huddle as a message with no text and one RICH_LINK
annotation (a MEET_SPACE link whose `meetSpaceLinkData` has type HUDDLE and a `huddleStatus` that moves from
STARTED to ENDED or MISSED when the message is updated). Google Meet cannot load in a frame, so the call runs in the real Meet in a small window: Chrome's app
window in the profile already signed in as this account, so nobody signs in again. Without such a profile it is an
always-on-top webview with its own per-account Google session, one sign-in per account, and no access to Confab's commands: the
header's Huddle button creates a Meet space through the Meet REST API (`create_meet`), posts that annotated
message (`post_huddle`, with a plain "Started a huddle" plus link as the fallback if Google rejects the annotation),
opens the mini window (the browser where the shell has none, or from the strip's move button, for screen sharing) and shows the strip at the foot of the sidebar, which polls `meet_status` (the
space's active conference and its participants) every eight seconds and closes itself when the call ends. No public Google API rings a person or starts a native huddle, so in a direct message or group
chat the Huddle button opens that conversation in Google Chat, where pressing Huddle rings them normally. In a named
space Confab creates the Meet and posts the message, with `<users/all>` up to 20 members so Google notifies them. A
huddle from anyone else rings here: a popup with Join and Decline for 40 seconds, the window comes forward and a
system notification fires, and it stops when Google marks the huddle ended or missed; do not disturb and muted spaces
do not ring; in Confab the message renders as a huddle card
with Join while it is live, the header offers Join huddle for a live one in the space, and the sidebar row shows a
headphone while one is on. This needs the Meet API enabled in the GCP project and the `meetings.space.created` and
`meetings.space.readonly` scopes (the stored sign-in asks for consent again). When Meet cannot be created, Confab
falls back to Confab voice: in-app audio between Confab clients over WebRTC signalled through the relay
(`huddle_join`, `huddle_leave`, `huddle_signal`; a full mesh, the lower-sorting name offers, STUN only), with mute
and speaking rings; the desktop grants the microphone to the webview (`on_permission_request`, `Info.plist` usage
strings). The mock plays both parts: a Meet stand-in whose status gains Priya after a few seconds, and a voice peer
that really answers with a quiet tone.

Calendar (sidebar row, also loaded at sign-in for the chip): Google Calendar through the Calendar v3 slice. Every
visible calendar's events in a window from five weeks back to ten weeks ahead, then each calendar's sync token
every thirty seconds while the view is open and every three minutes otherwise, so other people's changes land
within half a minute (an expired token reloads the window). Month, week, day and agenda views with a now line,
side-by-side overlaps, all-day rows, calendar toggles with Google's colours and a mini month. A popover per event
shows when, where, the description, guests with their answers, Join with Google Meet, Yes, Maybe, No, Edit, Open in
Google Calendar and Delete (organisers only). New event (or a double-click on a slot) opens the editor: title,
start and end or all day, repeat (daily, weekdays, weekly, monthly, yearly as RRULE), the calendar, guests from the
people picker, Google Meet, location, description; invites go out with `sendUpdates=all`. The top bar shows the
next meeting with a countdown and a Join once it is ten minutes away, and a notification fires ten minutes before
each timed event. Scopes `calendar.events` and `calendar.readonly`; the Calendar API must be enabled in the project.

Notes: Google Keep has no API for a signed-in user (it is enterprise-only through service accounts), so Confab's
notes are its own Keep-style notes stored as one JSON document in the user's Drive app data (`drive.appdata`
scope, the Drive API enabled in the project), which follows the account to any Confab. Quick composer, pinned and
others, colours, inline editing, search; saves a second after the last edit, last write wins.

Accounts side by side: the shell keeps one profile per Google account (its own token store,
cache and relay session) and the rail shows a tile per account above the views, the active one framed, unread from
the others as a badge, and a plus to add one. Switching (`account_switch`) reboots the workspace for that account;
adding (`account_add`) opens the sign-in screen with a way back; removing (`account_remove`) drops the profile from
this Mac only. Inactive accounts stay connected to the relay, count their new messages on the tile and send one
notification a minute at most. The workspace menu under the Confab title lists the other accounts too.

Status and presence go both ways. The rail menu pushes the choice to the relay for other Confab users and to Google
Chat through `users.availability` for the signed-in user (`availability_mark`: markAsActive, markAsAway,
markAsDoNotDisturb with a duration; `availability_status`: the custom status patch, which needs an emoji, 💬 when
none is given, and may clear itself after a chosen time), so Gmail and Chat show the same state. At sign-in Google's
view seeds the rail (`availability_get`). Google's ACTIVE expires after half an hour, so Confab refreshes it every
25 minutes while the window has focus. Google exposes only the signed-in user's availability, so other people's
dots still come from the relay. Scope `chat.users.availability`.

Space options (Google Chat's header): the space name opens a menu with Members, Add members, Files, Pinned, Tasks
from here, Start a video call, Star, Mute, Copy link, Open in Google Chat and Leave space. The header also carries a
Meet button (opens a fresh meeting at meet.google.com/new, since the API has no handle on a space's own call), Files
(a panel of every image and file shared in the space from the loaded messages, with Look further back), Tasks from
this space (Todos filtered to tasks whose notes carry a link into the space) and Pinned (a panel of pinned messages;
pins are local to this device because Google has no pin API for user apps). Adding members uses
`spaces.members.create`, which needs the `chat.memberships` scope (the stored sign-in is asked to consent again).
An empty space shows the welcome card with Add members, Share a file, Assign tasks and Start a video call. The
composer's + menu offers Upload a file, Pick from Google Drive, Start a video call and Create a calendar invite.

Message bodies render from `formattedText`, which is where Google puts the markup and links; `text` is the plain
body and is only a fallback. The docs say mentions arrive as `<users/id>`; the wire actually sends
`<chat-user data-user="users/id"></chat-user>`, and the parser reads both (any other `<chat-…>` wrapper loses its
tags and keeps its text). Mentions of you are also read from `annotations`, which is
how UI-typed mentions arrive.

Link previews ask first, because the shell fetches the page and the site sees your address: the Preferences setting
is Off, Ask (default) or Always. On Ask every link gets a chip; its menu offers this link once, the site from now
on (remembered, listed in Preferences with a remove button) or every link. The shell's `unfurl` command follows
http(s) only, refuses loopback, private and link-local hosts, caps at 512 KB and six seconds, and reads Open Graph
tags with a title and description fallback. Google's own apps are never previewed (their share cards cover it).

Todos (`⌘⇧T`) is Google Tasks' default list: add, tick, reopen, delete, due dates with overdue in red, and Add to
Tasks on any message's context menu (title from the first line, the message link in the notes). It needs the
`tasks` scope (`SCOPES` in confab-proto, so an older sign-in is asked to consent again) and the Tasks API enabled in
the GCP project. People lists everyone from the spaces you share (members load when the view opens) with presence,
email and a Message button; typing searches the Workspace directory as well.

Notifications (Preferences: All, DMs and mentions, Off) are system notifications through `notify`
(tauri-plugin-notification on desktop, the Notification API in browsers once allowed) for messages that land while
the space is not open or the window is not focused; never on Do not disturb or in a muted space. A `cargo tauri dev`
binary is not an app bundle, so macOS attributes them to Terminal and asks once.

Composer: `@` completes people, `:` completes emoji shortcodes (`:fi` → 🔥), a URL pasted over selected words links
them, files paste or drop. Sounds (Preferences) play a short synthesised two-note chime (`lib/sound.ts`, no asset)
when a message lands in a space you are not looking at, never on Do not disturb or in a muted space; WebKit only
starts audio after the first click or key. The people picker answers from loaded spaces at once and asks the
Workspace directory with an eight second cap; a directory that fails (the admin "Directory sharing" setting) says so
instead of spinning.

Attachments: images render inline through the `prism-media` proxy; everything else is a card with a kind badge
(PDF, DOC, SHEET, ZIP, ...). Drive shares from the Google Drive app arrive as `{name, source}` stubs and Google
refuses `attachments.get` for user auth, so the card is built from the share message's own `formattedText` link
(name, URL, kind from the docs/drive URL path, `driveDataRef.driveFileId` picks the right link when there are several).
Cards with an http(s) link open in the browser; uploaded files on desktop are saved to Downloads and revealed
(`save_attachment`). Files can be attached with the clip button, pasted, or dropped on the window (a drop zone shows
while files are dragged over it).

Motion: every transition is transform or opacity on the element itself, never on a glass surface. Tokens live on
`:root` (`--ease-out`, `--ease-spring`, `--t-fast`, `--t`, `--t-slow`), floating surfaces use the near-opaque `--pop`
and one rung of the z ladder (`--z-rail` < `--z-pop` < `--z-menu` < `--z-overlay` < `--z-emoji` < `--z-toast` <
`--z-drop`), and anything positioned `fixed` is portalled to `body` because virtual rows are transformed and clipped.
The window can go down to 880 by 560. Below 1240 px the header drops the facepile; below 1100 the Meet and Huddle
buttons keep only their icons, the sidebar caps at 240 px, side panels float over the list instead of squeezing it,
and the calendar folds its side column; below 980 the header's about line and the pinned and tasks buttons go, and
the top bar keeps only the search. Dialogs cap their height to the window and scroll their body, so Preferences stays reachable on a short screen.
Exit animations come from `useExit` in `lib/motion.ts`: the host keeps the element mounted and passes `exiting`.
Reduce motion in Preferences, or the OS setting, turns all of it off. One rule learnt the hard way: a hidden document
(an occluded window, a background tab) freezes CSS animations on their first frame, so while `document.hidden` the
root carries `data-hidden` and enter animations complete at once; without it every fade-in stays at opacity 0 until
the window is uncovered.

## Run locally
```
cargo run -p google-twin            # :8081, seeded workspace, prints the seeded users
cargo run -p relay                  # :8080
pnpm -C apps/desktop install && pnpm -C apps/desktop dev   # browser mode on :1420
cargo tauri dev --config apps/desktop/src-tauri/tauri.conf.json   # desktop
```
`docker compose up` runs twin + relay in containers.

## Connect to your Google Workspace (admin, about ten minutes)
Everything lives in a GCP project inside your Workspace org, so the consent screen can be **Internal** (no
verification, no scope registration). gcloud does all but two pages:

```
make gcp-setup PROJECT=my-project ACCOUNT=you@your-org
```
It signs gcloud in, creates the project under your org if missing, enables Chat, Workspace Events, Pub/Sub and
IAP, creates the Internal consent screen through the IAP brand API, the `prism-events` topic with
`chat-api-push@system.gserviceaccount.com` as publisher, the `prism-relay-pull` subscription, a `prism-relay`
service account with subscriber rights and a key under `~/.config/prism/`, and writes `.env.real` with the project
and key path filled in. It is idempotent, so rerun it after any failure.

Then the two pages Google has no API for (the script prints the direct links):
1. **OAuth client** of type **Desktop app**: paste its client id and secret into `.env.real`
   (`PRISM_CLIENT_ID`, `PRISM_CLIENT_SECRET`). Google requires the secret on the token endpoint even with PKCE;
   it is not treated as a secret.
2. **Google Chat API → Configuration**: app name, avatar URL, description; interactive features off. Without it,
   sending a message and creating an event subscription return 403 (reads work without it).

Run it:
```
make relay-real      # pulls prism-relay-pull with the service-account key, creates each user's Workspace Events subscription on first sign-in
make desktop-real    # sign in: Google's consent page opens in your browser
```
The relay log shows `subscription created` after the first sign-in; a message sent from the Google Chat web app
should then appear in Confab within a second or two.

The relay also accepts a gcloud ADC file (`GOOGLE_APPLICATION_CREDENTIALS` unset), but Workspace orgs with Google
Cloud session control reject its refresh token with `invalid_rapt`, which is why the script makes a service account.
Push delivery (a public HTTPS relay with an OIDC-verified
push subscription) is also implemented; its env vars are `PRISM_PUSH_AUDIENCE` and `PRISM_PUSH_SERVICE_ACCOUNT`.

## Local deploy
Everything runs on one machine, no cloud. Makefile targets:

| target | does |
| --- | --- |
| `make twin` / `make relay` | run the Google twin (:8081) / relay (:8080) with cargo |
| `make web` | Vite browser mode on :1420 |
| `make desktop` | `cargo tauri dev` |
| `make desktop-b` | second instance (`PRISM_PROFILE=b`) from the already-built `target/debug/prism-desktop` |
| `make up` / `make down` | `docker compose` twin + relay |
| `make check` | fmt, clippy, tests, TypeScript typecheck |
| `make build-desktop` | `cargo tauri build` |
| `make floci` | start the Floci GCP emulator and create the Pub/Sub topic + push subscription |
| `make e2e` | end-to-end check of typing, events, receipts and presence through the relay |
| `make gcp-setup PROJECT=id ACCOUNT=you@your-org` | one-shot real-GCP setup (APIs, consent screen, topic, pull subscription, `.env.real`) |
| `make relay-real` / `make desktop-real` | the same with `.env.real` loaded, against Google |

`docker compose up` builds both images from the workspace root and starts twin, relay, the Floci GCP emulator
(Pub/Sub on :4588) and a one-shot job that creates the `prism-events` topic and the push subscription to the relay.
Events then travel twin → Pub/Sub → relay, the same path as on real GCP.

Host flow with [Floci](https://floci.io) (`brew install floci-io/floci/floci`):
```
make floci        # floci gcp start, wait, create topic + push subscription to host.docker.internal:8080
PRISM_PUBSUB_EMULATOR_HOST=localhost:4588 PRISM_PUBSUB_TOPIC=projects/floci-local/topics/prism-events make twin
make relay
make e2e          # node tests/e2e-typing.mjs: typing, event, receipt and presence between two users
```
