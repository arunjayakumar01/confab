// Fixture bridge: VITE_BRIDGE=mock renders the whole app with nothing else running.
// note: in-memory only, ~60 messages, canned typing/presence timers, no pagination beyond 3 synthetic pages.
import type {
  Account, Attachment, AuthStatus, Availability, Bridge, CalEvent, CalendarEntry, ChatEvent, ClientMsg, EventsPage, LinkPreview, MeetSpace, MeetStatus, Membership, Message, Reaction, ServerMsg, ShellEvent, Space, Task, User,
} from "../bridge-types";
import { strToU8, zipSync } from "fflate";
import * as XLSX from "xlsx";
import { uuid } from "../lib/ids";
import { FOLDER, isGoogleFile } from "../lib/drive";
import type { DriveItem, DriveWhere, SharedDrive } from "../bridge-types";
import { webNotify } from "./browser";

const ME = "users/1001";
const USERS: Record<string, User> = {
  "users/1001": { name: "users/1001", displayName: "Arun J", email: "arun@example.test", type: "HUMAN" },
  "users/1002": { name: "users/1002", displayName: "Priya Nair", email: "priya@example.test", type: "HUMAN" },
  "users/1003": { name: "users/1003", displayName: "Rahul Mehta", email: "rahul@example.test", type: "HUMAN" },
  "users/1004": { name: "users/1004", displayName: "Sana Qureshi", email: "sana@example.test", type: "HUMAN" },
  "users/1005": { name: "users/1005", displayName: "Dev Patel", email: "dev@example.test", type: "HUMAN" },
  // The Google Drive app: it DMs you when someone shares a file. Not a person, so searchPeople skips it.
  "users/drive": { name: "users/drive", displayName: "Google Drive", type: "BOT" },
};
// Directory-only people: searchPeople finds them, no fixture space has them.
const DIRECTORY: User[] = [
  { name: "users/1006", displayName: "Meera Iyer", email: "meera@example.test", type: "HUMAN" },
  { name: "users/1007", displayName: "Tom Okafor", email: "tom@example.test", type: "HUMAN" },
];
const SPACES: Space[] = [
  { name: "spaces/AAAgeneral", spaceType: "SPACE", displayName: "general", spaceDetails: { description: "Team-wide announcements and the odd tangent" }, membershipCount: { joinedDirectHumanUserCount: 5 }, spaceThreadingState: "THREADED_MESSAGES" },
  { name: "spaces/AAAdesign", spaceType: "SPACE", displayName: "design", spaceDetails: { description: "Glass, type, motion. Screenshots welcome." }, membershipCount: { joinedDirectHumanUserCount: 4 }, spaceThreadingState: "THREADED_MESSAGES" },
  { name: "spaces/AAAinfra", spaceType: "SPACE", displayName: "infra-alerts", spaceDetails: { description: "Relay and twin health, deploy notes" }, membershipCount: { joinedDirectHumanUserCount: 3 }, spaceThreadingState: "THREADED_MESSAGES" },
  { name: "spaces/DMpriya", spaceType: "DIRECT_MESSAGE", membershipCount: { joinedDirectHumanUserCount: 2 } },
  { name: "spaces/DMrahul", spaceType: "DIRECT_MESSAGE", membershipCount: { joinedDirectHumanUserCount: 2 } },
  { name: "spaces/GRPcore", spaceType: "GROUP_CHAT", membershipCount: { joinedDirectHumanUserCount: 3 } },
  { name: "spaces/DMdrive", spaceType: "DIRECT_MESSAGE", membershipCount: { joinedDirectHumanUserCount: 1 } },
];
const MEMBERS: Record<string, string[]> = {
  "spaces/AAAgeneral": ["users/1001", "users/1002", "users/1003", "users/1004", "users/1005"],
  "spaces/AAAdesign": ["users/1001", "users/1002", "users/1004", "users/1005"],
  "spaces/AAAinfra": ["users/1001", "users/1003", "users/1005"],
  "spaces/DMpriya": ["users/1001", "users/1002"],
  "spaces/DMrahul": ["users/1001", "users/1003"],
  "spaces/GRPcore": ["users/1001", "users/1002", "users/1004"],
  "spaces/DMdrive": ["users/1001", "users/drive"],
};

const IMG =
  "data:image/svg+xml;utf8," +
  encodeURIComponent(
    `<svg xmlns="http://www.w3.org/2000/svg" width="640" height="360"><defs><linearGradient id="g" x1="0" x2="1" y1="0" y2="1"><stop offset="0" stop-color="#7c6cff"/><stop offset="0.6" stop-color="#ffb199"/><stop offset="1" stop-color="#2ec4b6"/></linearGradient></defs><rect width="640" height="360" fill="url(#g)"/><rect x="24" y="24" width="68" height="312" rx="14" fill="rgba(20,16,40,0.6)"/><rect x="108" y="24" width="220" height="312" rx="14" fill="rgba(255,255,255,0.45)"/><rect x="344" y="24" width="272" height="312" rx="14" fill="rgba(255,255,255,0.6)"/><text x="360" y="70" font-family="system-ui" font-size="18" fill="#1d1c1d">glass rail, first pass</text></svg>`,
  );

// A tiny but well-formed PDF, so a file card has something real behind its download link.
const PDF = "data:application/pdf;base64,JVBERi0xLjQKMSAwIG9iago8PCAvVHlwZSAvQ2F0YWxvZyAvUGFnZXMgMiAwIFIgPj4KZW5kb2JqCjIgMCBvYmoKPDwgL1R5cGUgL1BhZ2VzIC9LaWRzIFszIDAgUl0gL0NvdW50IDEgPj4KZW5kb2JqCjMgMCBvYmoKPDwgL1R5cGUgL1BhZ2UgL1BhcmVudCAyIDAgUiAvTWVkaWFCb3ggWzAgMCA2MTIgNzkyXSAvQ29udGVudHMgNCAwIFIgL1Jlc291cmNlcyA8PCAvRm9udCA8PCAvRjEgNSAwIFIgPj4gPj4gPj4KZW5kb2JqCjQgMCBvYmoKPDwgL0xlbmd0aCAyNTIgPj4Kc3RyZWFtCkJUIC9GMSAyOCBUZiA3MiA3MDAgVGQgKE1vdGlvbiBzcGVjLCB2MikgVGogRVQgQlQgL0YxIDE0IFRmIDcyIDY2MCBUZCAoRXZlcnkgdHJhbnNpdGlvbiBpcyB0cmFuc2Zvcm0gb3Igb3BhY2l0eSBvbiB0aGUgZWxlbWVudCBpdHNlbGYuKSBUaiBFVCBCVCAvRjEgMTQgVGYgNzIgNjQwIFRkIChGbG9hdGluZyBzdXJmYWNlcyB1c2UgdGhlIG5lYXItb3BhcXVlIHBvcCBzdXJmYWNlIGFuZCBvbmUgcnVuZyBvZiB0aGUgeiBsYWRkZXIuKSBUaiBFVAplbmRzdHJlYW0KZW5kb2JqCjUgMCBvYmoKPDwgL1R5cGUgL0ZvbnQgL1N1YnR5cGUgL1R5cGUxIC9CYXNlRm9udCAvSGVsdmV0aWNhID4+CmVuZG9iagp4cmVmCjAgNgowMDAwMDAwMDAwIDY1NTM1IGYgCjAwMDAwMDAwMDkgMDAwMDAgbiAKMDAwMDAwMDA1OCAwMDAwMCBuIAowMDAwMDAwMTE1IDAwMDAwIG4gCjAwMDAwMDAyNDEgMDAwMDAgbiAKMDAwMDAwMDU0NCAwMDAwMCBuIAp0cmFpbGVyCjw8IC9TaXplIDYgL1Jvb3QgMSAwIFIgPj4Kc3RhcnR4cmVmCjYxNAolJUVPRgo=";
const MOTION_SPEC: Attachment = { contentName: "motion-spec.pdf", contentType: "application/pdf", downloadUri: PDF, source: "UPLOADED_CONTENT" };

// Office and archive fixtures built on the fly, so the viewers have real bytes to open.
const b64 = (u8: Uint8Array): string => { let bin = ""; for (const b of u8) bin += String.fromCharCode(b); return btoa(bin); };
const DOCX_XML = (body: string) => `<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>${body}<w:sectPr/></w:body></w:document>`;
const para = (t: string, style?: string) => `<w:p>${style ? `<w:pPr><w:pStyle w:val="${style}"/></w:pPr>` : ""}<w:r><w:t xml:space="preserve">${t}</w:t></w:r></w:p>`;
const DOCX_BYTES = zipSync({
  "[Content_Types].xml": strToU8(`<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>`),
  "_rels/.rels": strToU8(`<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>`),
  "word/document.xml": strToU8(DOCX_XML(para("Motion review notes", "Heading1") + para("The glass chrome holds up at 60 fps on the M2; the rail blur is the only thing that drops frames on the 2019 Intel.") + para("Decisions", "Heading2") + para("Keep the spring curve for pops, use ease-out everywhere else.") + para("Fold anything past sixteen lines; the mask fade reads better than a gradient over glass."))),
});
const SHEET_BYTES = (() => {
  const wb = XLSX.utils.book_new();
  XLSX.utils.book_append_sheet(wb, XLSX.utils.aoa_to_sheet([["Item", "Owner", "Status", "Due"], ["Relay reconnect", "Priya", "Done", "2026-11-04"], ["Twin search", "Rahul", "In review", "2026-11-07"], ["Drive cards", "Arun", "Done", "2026-11-06"], ["Notifications", "Dev", "Open", "2026-11-09"]]), "Sprint 45");
  XLSX.utils.book_append_sheet(wb, XLSX.utils.aoa_to_sheet([["Month", "Spend (USD)"], ["Sep", 412], ["Oct", 388], ["Nov", 140]]), "Cloud spend");
  return new Uint8Array(XLSX.write(wb, { type: "array", bookType: "xlsx" }) as ArrayBuffer);
})();
const ZIP_BYTES = zipSync({
  "release-notes/README.md": strToU8("# Confab 0.1\n\nMotion, layering and the file viewer.\n"),
  "release-notes/notes.docx": DOCX_BYTES,
  "release-notes/sprint.xlsx": SHEET_BYTES,
  "release-notes/motion-spec.pdf": new Uint8Array(atob(PDF.split(",")[1]!).split("").map((c) => c.charCodeAt(0))),
  "release-notes/assets/glass-rail.svg": strToU8(decodeURIComponent(IMG.split(",")[1]!)),
});
const OFFICE_FILES: Attachment[] = [
  { contentName: "motion-review.docx", contentType: "application/vnd.openxmlformats-officedocument.wordprocessingml.document", source: "UPLOADED_CONTENT", downloadUri: "data:application/vnd.openxmlformats-officedocument.wordprocessingml.document;base64," + b64(DOCX_BYTES) },
  { contentName: "sprint-45.xlsx", contentType: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet", source: "UPLOADED_CONTENT", downloadUri: "data:application/vnd.openxmlformats-officedocument.spreadsheetml.sheet;base64," + b64(SHEET_BYTES) },
  { contentName: "release-notes.zip", contentType: "application/zip", source: "UPLOADED_CONTENT", downloadUri: "data:application/zip;base64," + b64(ZIP_BYTES) },
];

// [sender, minutesAgo, text, extras]
//   image: the glass-rail PNG (full resource inline)
//   file:  UPLOADED_CONTENT attachments, full resources inline (name filled in when missing)
//   drive: the full Drive resource; the message itself carries only {name, source}, like real Google,
//          and getAttachment resolves the rest
type Fx = [string, number, string, {
  thread?: string; reply?: string; reactions?: [string, number][]; mine?: string[];
  image?: boolean; file?: Attachment[]; drive?: Omit<Attachment, "name">;
  deleted?: string; // deletionType: the row is a tombstone
}?];
const FIXTURES: Record<string, Fx[]> = {
  "spaces/AAAgeneral": [
    ["users/1002", 2880, "Morning all. Twin and relay both come up from `docker compose up` now, so you can run the whole thing on one laptop."],
    ["users/1003", 2875, "Nice. Does the twin seed users or do I create them?", { reactions: [["👀", 1]] }],
    ["users/1002", 2870, "Seeded. It prints them on boot, `users/1001` is you.", { thread: "seed" }],
    ["users/1003", 2860, "Perfect, thanks", { reply: "seed" }],
    ["users/1004", 2850, "Which one is me?", { reply: "seed" }],
    ["users/1002", 2845, "1004 :)", { reply: "seed" }],
    ["users/1003", 2700, "", { deleted: "CREATOR" }],
    ["users/1004", 1600, "Accessibility pass, what landed this week\nCommon to all five portals\nSkip to main content link, visible keyboard focus, accessible names on icon controls and form fields\n1. Landing pages, https://example.test/ (repo site)\nFixes: skip link, main and navigation landmarks, keyboard focus; white text on the DPG card; text alternative for the home page video.\n395ebf9 skip link, landmarks, focus https://example.test/site/commit/395ebf9\ndbf1956 DPG card contrast https://example.test/site/commit/dbf1956\n2a52f60 video text alternative https://example.test/site/commit/2a52f60\n2. Yield Estimation, https://example.test/cropyield/ (repo yield-dashboard)\nFixes: named selectors, page heading, skip link; darker toggles and buttons; Excel export shows file type and size.\ne1083cf selectors named, h1, skip link https://example.test/yield/commit/e1083cf\n2f17b4c contrast, export type and size https://example.test/yield/commit/2f17b4c\n3. Weather, https://example.test/weather/\n> Still open: the map canvas has no text alternative; waiting on the tiler team.\nThe contrast check we ran, for the record:\n```sh\nnpx pa11y https://example.test/ --standard WCAG2AA\nnpx axe https://example.test/cropyield/ --tags wcag2aa\n```\n- 8 criteria met\n- 19 gaps, 11 of them fixed above\n- the rest are tracked in [the sheet](https://docs.google.com/spreadsheets/d/1abc/edit)\nShout if any of the commits look wrong."],
    ["users/1005", 1500, "Deploy notes for the week are in <https://example.test/notes/w45|the wiki>. Nothing scary."],
    ["users/1001", 1495, "Thanks <users/1005>. Reading now."],
    ["users/1005", 1494, "There is one thing: **the relay dedupes events for 60 s** by `message.name + lastUpdateTime`, so a quick edit-after-edit may collapse.", { reactions: [["👍", 2]], mine: ["👍"] }],
    ["users/1004", 1480, "That matches what Google does anyway", { reactions: [["💯", 1]] }],
    ["users/1002", 240, "Standup in 10. Agenda:\n- glass chrome review\n- relay reconnect backoff\n- twin search endpoint"],
    ["users/1003", 238, "Can we add the PKCE loopback port to that? I keep hitting a clash on 1420."],
    ["users/1002", 237, "Added"],
    ["users/1004", 236, "+1", { reactions: [["🙏", 1]] }],
    ["users/1002", 60, "> standup moved to 11\nsorry, dentist", { reactions: [["🦷", 2], ["😂", 1]] }],
    ["users/1001", 58, "no problem"],
    ["users/1003", 12, "Relay is back up on :8080, reconnect with backoff works from the web view too.", { reactions: [["🎉", 3]], mine: ["🎉"], thread: "relay" }],
    ["users/1001", 10, "What was the cap on the backoff?", { reply: "relay" }],
    ["users/1003", 9, "30 s, doubling from 1 s", { reply: "relay" }],
    ["users/1004", 3, "<users/all> design review link is in #design"],
  ],
  "spaces/AAAdesign": [
    ["users/1004", 4300, "First pass at the glass rail. The sidebar reads better at 55% white than 70%.", { image: true, reactions: [["🔥", 3], ["👀", 2]], mine: ["🔥"] }],
    ["users/1002", 4290, "Agree. The hairline border carries the edge, the fill can stay quiet."],
    ["users/1001", 4280, "Does it hold up in dark mode?", { thread: "dark" }],
    ["users/1004", 4270, "Yes, same tint at 62% over the vibrancy. Rail stays dark in both.", { reply: "dark" }],
    ["users/1005", 4260, "The traffic lights need 16px / 18px on macOS, do not forget the drag region.", { reply: "dark" }],
    ["users/1004", 1700, "Type scale: 15px body, 13px meta, 18px header. _System stack_, Inter if installed."],
    ["users/1002", 1690, "~14px body~ 15px, agreed", { reactions: [["👍", 2]] }],
    ["users/1005", 1680, "Code blocks:\n```ts\nconst glass = `blur(20px) saturate(180%)`;\n```\nthat is the whole trick"],
    ["users/1004", 400, "Unread badge colour: red, or accent?"],
    ["users/1001", 395, "Red. The accent is already doing the active state."],
    ["users/1004", 394, "ok ✅"],
    ["users/1002", 90, "Reduced transparency fallback is solid now, the blur just switches off. Spec attached.", { file: [MOTION_SPEC] }],
    ["users/1004", 30, "Review at 3? <users/1001> <users/1002>"],
    ["users/1002", 28, "yes"],
  ],
  "spaces/AAAinfra": [
    ["users/1005", 3000, "*relay* restarted after the SQLite migration. No data lost."],
    ["users/1003", 2990, "ack"],
    ["users/1005", 1200, "Pub/Sub push verified against the Google certs endpoint. Twin mode still uses the shared secret header."],
    ["users/1005", 700, "Heads up: `PRISM_DB` default is `relay.db` in the cwd. Set it in the compose file."],
    ["users/1003", 690, "done, it is under /data now", { reactions: [["👍", 1]], mine: ["👍"] }],
    ["users/1005", 120, "Workspace Events subscription renewed, expires in 7 days."],
    ["users/1003", 45, "Do we auto-renew?"],
    ["users/1005", 44, "Yes, the manager renews at 24 h before expiry."],
  ],
  "spaces/DMpriya": [
    ["users/1002", 5000, "hey, got a sec?"],
    ["users/1001", 4995, "sure"],
    ["users/1002", 4990, "can you look at the receipt fan-out? I think we send it to the sender too"],
    ["users/1001", 4980, "yeah, it goes to every subscriber including you. harmless but noisy"],
    ["users/1002", 4975, "ok I will filter it on the client then"],
    ["users/1002", 180, "lunch?"],
    ["users/1001", 175, "12:30 works"],
    ["users/1002", 170, "👍"],
    ["users/1001", 20, "the thread panel slides in now, have a look when you are back"],
    ["users/1002", 5, "on it"],
  ],
  "spaces/DMrahul": [
    ["users/1003", 6000, "The twin returns 409 on a duplicate messageId, so retries are safe."],
    ["users/1001", 5990, "good, the outbox relies on that"],
    ["users/1003", 300, "ping me when the search endpoint is wired"],
    ["users/1003", 295, "the review pack: notes, the sprint sheet and everything zipped", { file: OFFICE_FILES }],
    ["users/1005", 290, "relay log from the drop, for the record", { file: [{ contentName: "relay.log", contentType: "text/plain", source: "UPLOADED_CONTENT", downloadUri: "data:text/plain;base64," + btoa("2026-11-06T17:36:41Z INFO relay::ws session closed session=5df98b9b user=users/1001\n2026-11-06T17:37:45Z WARN relay::subs membership denied space=spaces/AAAinfra\n2026-11-06T17:47:11Z INFO relay::subs subscription renewed name=subscriptions/chat-spaces-czo\n2026-11-06T17:48:16Z INFO relay::ws session open session=168c235e user=users/1001\n") }] }],
    ["users/1001", 290, "will do"],
    ["users/1003", 15, "also, Alt+Shift+Up for previous unread is lovely"],
  ],
  "spaces/GRPcore": [
    ["users/1004", 2000, "core trio sync: are we keeping the 68px rail?"],
    ["users/1002", 1995, "yes, muscle memory"],
    ["users/1001", 1990, "same"],
    ["users/1004", 1985, "ok. and the thread panel at 400?"],
    ["users/1002", 1980, "400 is fine on a 13 inch"],
    ["users/1004", 50, "pushed the member list panel, it shares the slot with threads"],
    ["users/1002", 48, "nice 🚀", { reactions: [["🚀", 2]], mine: ["🚀"] }],
  ],
  "spaces/DMdrive": [
    ["users/drive", 2600, "Priya Nair shared a file with you", { drive: {
      contentName: "Q4 roadmap.pdf", contentType: "application/pdf", source: "DRIVE_FILE",
      driveDataRef: { driveFileId: "1abcQ4RoadmapX7kL0pQ9mN2vB4cD6eF8g" },
      downloadUri: "https://drive.google.com/file/d/1abcQ4RoadmapX7kL0pQ9mN2vB4cD6eF8g/view",
    } }],
    ["users/drive", 900, "Sana Qureshi shared a document with you", { drive: {
      contentName: "Glass chrome spec", contentType: "application/vnd.google-apps.document", source: "DRIVE_FILE",
      driveDataRef: { driveFileId: "1defGlassChromeSpecR3tY5uI7oP9aS1dF" },
      downloadUri: "https://docs.google.com/document/d/1defGlassChromeSpecR3tY5uI7oP9aS1dF/edit",
    } }],
    ["users/drive", 35, "Dev Patel shared a spreadsheet with you", { drive: {
      contentName: "Relay latency budget", contentType: "application/vnd.google-apps.spreadsheet", source: "DRIVE_FILE",
      driveDataRef: { driveFileId: "1ghiRelayLatencyBudgetZ2xC4vB6nM8qW" },
      downloadUri: "https://docs.google.com/spreadsheets/d/1ghiRelayLatencyBudgetZ2xC4vB6nM8qW/edit",
    } }],
  ],
};

function build(now: number): { store: Map<string, Message[]>; drive: Map<string, Attachment> } {
  const store = new Map<string, Message[]>();
  const drive = new Map<string, Attachment>(); // full Drive resources by attachment name
  let n = 0;
  for (const [space, rows] of Object.entries(FIXTURES)) {
    const threads: Record<string, string> = {};
    const msgs: Message[] = [];
    for (const [sender, minutesAgo, text, x] of rows) {
      n += 1;
      const name = `${space}/messages/m${n}`;
      let threadName = `${space}/threads/t${n}`;
      let threadReply = false;
      if (x?.thread) threads[x.thread] = threadName;
      if (x?.reply) {
        threadName = threads[x.reply] ?? threadName;
        threadReply = true;
      }
      const m: Message = {
        name,
        sender: USERS[sender],
        createTime: new Date(now - minutesAgo * 60_000).toISOString(),
        text,
        thread: { name: threadName },
        threadReply,
        clientAssignedMessageId: undefined,
      };
      if (x?.deleted) { delete m.text; m.deleteTime = new Date(now - (minutesAgo - 1) * 60_000).toISOString(); m.deletionMetadata = { deletionType: x.deleted }; }
      if (x?.reactions) m.emojiReactionSummaries = x.reactions.map(([u, c]) => ({ emoji: { unicode: u }, reactionCount: c }));
      if (x?.mine) m._myReactions = x.mine;
      if (x?.image) m.attachment = [{ name: `${name}/attachments/a1`, contentName: "glass-rail.png", contentType: "image/png", downloadUri: IMG, thumbnailUri: IMG, source: "UPLOADED_CONTENT" }];
      if (x?.file) m.attachment = [...(m.attachment ?? []), ...x.file.map((a, i) => ({ name: `${name}/attachments/f${i + 1}`, ...a }))];
      if (x?.drive) {
        // As on real Google: the listed message carries only name + source; getAttachment resolves the rest.
        const an = `${name}/attachments/d1`;
        drive.set(an, { name: an, ...x.drive });
        m.attachment = [...(m.attachment ?? []), { name: an, source: "DRIVE_FILE" }];
      }
      msgs.push(m);
    }
    store.set(space, msgs);
  }
  return { store, drive };
}

// Two accounts side by side: the fixture org, and a second org that sees only a couple of its spaces.
const ACCOUNTS: Account[] = [
  { id: "default", email: "arun@example.test", displayName: "Arun J", signedIn: true, active: true, unread: 0 },
  { id: "acct-2", email: "arun@example.test", displayName: "Arun Jayakumar", signedIn: true, active: false, unread: 2 },
];
let activeAccount = "default";
const accountSpaces = (list: Space[]): Space[] => (activeAccount === "acct-2" ? list.filter((s) => s.name === "spaces/AAAinfra" || s.name === "spaces/DMrahul") : list);

// Google Chat availability stand-in for the signed-in user.
let availability: Availability = { name: "users/me/availability", state: "ACTIVE" };

// Google Calendar stand-in: two calendars, events around today, a counter as the sync token.
const CALENDARS: CalendarEntry[] = [
  { id: "primary", summary: "Arun J", primary: true, backgroundColor: "#7986cb", accessRole: "owner" },
  { id: "team@example.test", summary: "Team", backgroundColor: "#33b679", accessRole: "reader" },
];
const at = (dayOffset: number, h: number, m = 0) => { const d = new Date(); d.setDate(d.getDate() + dayOffset); d.setHours(h, m, 0, 0); return d.toISOString(); };
let calClock = 1;
let evSeq = 0;
const calEvents = new Map<string, CalEvent & { _clock: number }>();
const seedEvent = (calendarId: string, e: Partial<CalEvent> & { summary: string; start: string; end: string }) => {
  const id = `ev-${++evSeq}`;
  calEvents.set(id, { id, calendarId, status: "confirmed", htmlLink: `https://calendar.google.com/event?eid=${id}`, created: at(-7, 9), updated: at(-1, 9), ...e, start: { dateTime: e.start }, end: { dateTime: e.end }, _clock: calClock });
};
seedEvent("primary", { summary: "Standup", start: at(0, 10), end: at(0, 10, 15), hangoutLink: "https://meet.google.com/abc-defg-hij", attendees: [{ email: "arun@example.test", displayName: "Arun J", self: true, organizer: true, responseStatus: "accepted" }, { email: "priya@example.test", displayName: "Priya Nair", responseStatus: "accepted" }, { email: "rahul@example.test", displayName: "Rahul Mehta", responseStatus: "needsAction" }], organizer: { email: "arun@example.test", self: true } });
seedEvent("primary", { summary: "Design review", description: "Glass chrome, motion tokens, the file viewer. Bring screenshots.", location: "Meet", start: at(0, new Date().getHours() + 2), end: at(0, new Date().getHours() + 2, 45), conferenceData: { entryPoints: [{ entryPointType: "video", uri: "https://meet.google.com/rev-iew0-one" }] }, attendees: [{ email: "arun@example.test", self: true, responseStatus: "needsAction" }, { email: "sana@example.test", displayName: "Sana Qureshi", organizer: true, responseStatus: "accepted" }, { email: "dev@example.test", displayName: "Dev Patel", responseStatus: "tentative" }], organizer: { email: "sana@example.test", displayName: "Sana Qureshi" } });
seedEvent("primary", { summary: "Relay deploy window", description: "Ship relay 0.4 to the shared box.", start: at(1, 15), end: at(1, 16) });
seedEvent("primary", { summary: "1:1 with Priya", start: at(2, 11), end: at(2, 11, 30), attendees: [{ email: "arun@example.test", self: true, organizer: true, responseStatus: "accepted" }, { email: "priya@example.test", displayName: "Priya Nair", responseStatus: "accepted" }] });
seedEvent("team@example.test", { summary: "All hands", start: at(3, 11), end: at(3, 12), location: "Town hall" });
seedEvent("team@example.test", { summary: "Sprint 46 planning", start: at(4, 14), end: at(4, 15, 30) });
seedEvent("primary", { summary: "Dentist", start: at(-1, 9), end: at(-1, 9, 45) });

// Notes stand-in: Keep-style notes as one JSON document, like the Drive app data file.
let notesJson: string | null = JSON.stringify({ v: 1, notes: [
  { id: "n1", title: "Relay release checklist", body: "- bump CLIENT_VERSION\n- rotate the push secret\n- watch the membership denied line for an hour", color: "sun", pinned: true, updated: at(-1, 17) },
  { id: "n2", title: "", body: "Idea: the New line could glow once when the space opens, then settle.", color: "mint", pinned: false, updated: at(-2, 12) },
  { id: "n3", title: "Standup notes", body: "Priya: relay reconnect done. Rahul: twin search. Me: Drive cards, file viewers.", color: "none", pinned: false, updated: at(0, 9) },
] });

// Google Meet stand-in: a meeting code per huddle, Priya joins the call a few seconds in.
let meetSeq = 0;
const meets = new Map<string, { at: number; message?: Message; space?: string }>();
const meetCode = (n: number) => { const a = "abcdefghijklmnopqrstuvwxyz"; const l = (i: number) => a[(n * 7 + i * 3) % 26]!; return `${l(0)}${l(1)}${l(2)}-${l(3)}${l(4)}${l(5)}${l(6)}-${l(7)}${l(8)}${l(9)}`; };

// A huddle peer that really answers: Priya's side of the call, a quiet tone instead of a microphone.
let huddleUsers: string[] = [];
let fakePeer: RTCPeerConnection | null = null;
let fakeTone: AudioContext | null = null;
async function answerAsPriya(space: string, p: { kind: string; sdp?: string; candidate?: RTCIceCandidateInit | null }): Promise<void> {
  const emitSig = (payload: unknown) => emitRef?.({ type: "huddle_signal", space, from: "users/1002", payload });
  try {
    if (p.kind === "offer" && p.sdp) {
      fakePeer?.close();
      const pc = new RTCPeerConnection({ iceServers: [] });
      fakePeer = pc;
      fakeTone ??= new AudioContext();
      const osc = fakeTone.createOscillator();
      const gain = fakeTone.createGain();
      gain.gain.value = 0.02;
      osc.frequency.value = 220;
      const dest = fakeTone.createMediaStreamDestination();
      osc.connect(gain).connect(dest);
      osc.start();
      for (const t of dest.stream.getAudioTracks()) pc.addTrack(t, dest.stream);
      pc.onicecandidate = (e) => emitSig({ kind: "ice", candidate: e.candidate ? e.candidate.toJSON() : null });
      await pc.setRemoteDescription({ type: "offer", sdp: p.sdp });
      const answer = await pc.createAnswer();
      await pc.setLocalDescription(answer);
      emitSig({ kind: "answer", sdp: answer.sdp });
    } else if (p.kind === "ice" && p.candidate && fakePeer?.remoteDescription) {
      await fakePeer.addIceCandidate(p.candidate);
    }
  } catch (e) {
    console.warn("mock huddle peer", e);
  }
}
let emitRef: ((m: ServerMsg | ShellEvent) => void) | null = null;
function endMockHuddle(code: string): void {
  const rec = meets.get(code);
  if (!rec?.message || !rec.space) return;
  const d = rec.message.annotations?.[0]?.richLinkMetadata?.meetSpaceLinkData;
  if (!d || d.huddleStatus === "ENDED") return;
  d.huddleStatus = "ENDED";
  rec.message.lastUpdateTime = new Date().toISOString();
  emitRef?.({ type: "event", event: { event_type: "google.workspace.chat.message.v1.updated", space: rec.space, event_time: new Date().toISOString(), message: rec.message } });
}

// Google Tasks stand-in: the default list, in memory.
const tomorrow = new Date(Date.now() + 86_400_000).toISOString().slice(0, 10) + "T00:00:00.000Z";
let taskSeq = 2;
const tasks: Task[] = [
  { id: "task-1", title: "Review the relay reconnect PR", notes: "From Priya Nair in Confab", status: "needsAction", due: tomorrow, updated: new Date(Date.now() - 3_600_000).toISOString(), webViewLink: "https://tasks.google.com/" },
  { id: "task-2", title: "Send the design spec to Priya", status: "needsAction", updated: new Date(Date.now() - 7_200_000).toISOString(), webViewLink: "https://tasks.google.com/" },
];

const CANNED = [
  "did anyone else see the relay drop for a second?",
  "ok that was me restarting it, sorry",
  "the day dividers look right now, Monday reads as Monday",
  "mock mode is such a time saver",
  "pushing the member panel fix in a minute",
  "lunch is here 🌮",
];

export function createMockBridge(): Bridge {
  const now = Date.now();
  const { store, drive } = build(now);
  const listeners = new Set<(m: ServerMsg | ShellEvent) => void>();
  const emit = (m: ServerMsg | ShellEvent) => listeners.forEach((l) => l(m));
  emitRef = emit;
  // which of my reactions exist, by message name
  const mine = new Map<string, Set<string>>();
  for (const msgs of store.values()) for (const m of msgs) if (m._myReactions) mine.set(m.name, new Set(m._myReactions));
  let seq = 1000;
  let timers: number[] = [];
  const signedOut = () => localStorage.getItem("prism.mock.signedOut") === "1";
  const later = (ms: number, f: () => void) => timers.push(window.setTimeout(f, ms));

  function findMessage(name: string): { list: Message[]; i: number } | null {
    for (const list of store.values()) {
      const i = list.findIndex((m) => m.name === name);
      if (i >= 0) return { list, i };
    }
    return null;
  }
  function event(space: string, event_type: string, extra: Partial<ChatEvent>): ServerMsg {
    return { type: "event", event: { event_type: `google.workspace.chat.${event_type}`, space, event_time: new Date().toISOString(), ...extra } };
  }
  function startTimers() {
    if (timers.length) return;
    later(300, () => {
      emit({ type: "relay_state", state: "open" });
      emit({ type: "welcome", user: USERS[ME]!, session_id: uuid(), online: ["users/1002", "users/1004", "users/1005"] });
      emit({ type: "presence", user: "users/1003", state: "away", at: new Date().toISOString() });
      emit({ type: "presence", user: "users/1005", state: "dnd", custom_status: { emoji: "🎧", text: "Heads down until 4" }, at: new Date().toISOString() });
      // Read receipts other Confab clients would send: Priya and Sana have read to the end of #general, Dev a little behind.
      const gen = [...(store.get("spaces/AAAgeneral") ?? [])].filter((m) => !m.threadReply).sort((a, b) => (a.createTime! < b.createTime! ? -1 : 1));
      const lastGen = gen[gen.length - 1];
      const prevGen = gen[gen.length - 3];
      if (lastGen) for (const u of ["users/1002", "users/1004"]) emit({ type: "receipt", space: "spaces/AAAgeneral", message: lastGen.name, user: u, at: new Date(Date.now() - 600_000).toISOString() });
      if (prevGen) emit({ type: "receipt", space: "spaces/AAAgeneral", message: prevGen.name, user: "users/1005", at: new Date(Date.now() - 3_600_000).toISOString() });
    });
    let k = 0;
    const tick = () => {
      const targets = ["spaces/AAAgeneral", "spaces/DMpriya", "spaces/GRPcore", "spaces/AAAdesign"];
      const space = targets[k % targets.length]!;
      const others = MEMBERS[space]!.filter((u) => u !== ME);
      const user = others[k % others.length]!;
      const text = CANNED[k % CANNED.length]!;
      k += 1;
      emit({ type: "typing", space, user, active: true, ttl_ms: 5000 });
      later(2500, () => emit({ type: "typing", space, user, active: true, ttl_ms: 5000 }));
      later(4500, () => {
        emit({ type: "typing", space, user, active: false, ttl_ms: 5000 });
        const list = store.get(space)!;
        const name = `${space}/messages/srv${++seq}`;
        const m: Message = { name, sender: USERS[user], createTime: new Date().toISOString(), text, thread: { name: `${space}/threads/t${seq}` }, threadReply: false };
        list.push(m);
        emit(event(space, "message.v1.created", { message: m }));
      });
      timers.push(window.setTimeout(tick, 14_000));
    };
    timers.push(window.setTimeout(tick, 6000));
    const states = ["active", "away", "dnd", "active"] as const;
    let p = 0;
    timers.push(window.setInterval(() => {
      const user = ["users/1002", "users/1003", "users/1004"][p % 3]!;
      emit({ type: "presence", user, state: states[(p >> 1) % states.length]!, at: new Date().toISOString() });
      p += 1;
    }, 20_000));
  }
  function stopTimers() {
    timers.forEach((t) => { window.clearTimeout(t); window.clearInterval(t); });
    timers = [];
  }

  const delay = (ms = 120) => new Promise<void>((r) => setTimeout(r, ms));
  const driveApi = mockDrive();

  const bridge: Bridge = {
    async authStatus(): Promise<AuthStatus> {
      await delay(50);
      const acct = ACCOUNTS.find((a) => a.id === activeAccount);
      if (acct && !acct.signedIn) return { signedIn: false };
      if (signedOut()) return { signedIn: false };
      startTimers();
      // The second account is the same person under another org's identity.
      return { signedIn: true, user: activeAccount === "acct-2" ? { ...USERS[ME]!, displayName: "Arun Jayakumar", email: "arun@example.test" } : USERS[ME]! };
    },
    async authStart() {
      await delay(600);
      localStorage.removeItem("prism.mock.signedOut");
      startTimers();
      return { signedIn: true, user: USERS[ME]! };
    },
    async signOut() {
      localStorage.setItem("prism.mock.signedOut", "1");
      stopTimers();
    },
    async listSpaces() {
      await delay();
      return accountSpaces(SPACES).map((s) => ({ ...s }));
    },
    async listMembers(space): Promise<Membership[]> {
      await delay();
      return (MEMBERS[space] ?? []).map((u) => ({ name: `${space}/members/${u.split("/")[1]}`, state: "JOINED", role: "ROLE_MEMBER", member: USERS[u] ?? DIRECTORY.find((d) => d.name === u) }));
    },
    async listMessages(space, opts = {}) {
      await delay(200);
      const all = store.get(space) ?? [];
      if (opts.thread) {
        const items = all.filter((m) => m.thread?.name === opts.thread).sort((a, b) => (a.createTime! < b.createTime! ? 1 : -1));
        return { items };
      }
      const page = Number(opts.pageToken?.split(":")[1] ?? 0);
      if (page === 0) {
        const items = [...all].sort((a, b) => (a.createTime! < b.createTime! ? 1 : -1));
        const deep = space === "spaces/AAAgeneral" || space === "spaces/AAAdesign";
        return deep ? { items, nextPageToken: "older:1" } : { items };
      }
      // Synthetic history so reverse infinite scroll has something to load.
      const oldest = all.reduce((t, m) => Math.min(t, new Date(m.createTime!).getTime()), now);
      const users = MEMBERS[space]!;
      const items: Message[] = [];
      for (let i = 0; i < 20; i++) {
        const idx = (page - 1) * 20 + i;
        items.push({
          name: `${space}/messages/old${page}-${i}`,
          sender: USERS[users[idx % users.length]!],
          createTime: new Date(oldest - (idx + 1) * 47 * 60_000 - page * 86_400_000).toISOString(),
          text: idx % 7 === 3 ? `older note ${idx}, with a **bold** bit and a link https://example.test/${idx}` : `older message ${idx}`,
          thread: { name: `${space}/threads/old${page}-${i}` },
          threadReply: false,
        });
      }
      return page < 3 ? { items, nextPageToken: `older:${page + 1}` } : { items };
    },
    async sendMessage(space, text, opts = {}) {
      const clientId = opts.clientId ?? uuid();
      const optimistic: Message = {
        name: `${space}/messages/client-${clientId}`,
        sender: USERS[ME],
        createTime: new Date().toISOString(),
        text,
        clientAssignedMessageId: `client-${clientId}`,
        _pending: "sending",
        ...(opts.thread ? { thread: { name: opts.thread }, threadReply: true } : {}),
        ...(opts.attachments?.length ? { attachment: opts.attachments } : {}),
      };
      later(500, () => {
        if (/\bfail\b/i.test(text)) {
          emit({ type: "outbox", clientId, status: "failed", error: "Twin said 429: slow down" });
          return;
        }
        const id = ++seq;
        const message: Message = {
          ...optimistic,
          name: `${space}/messages/srv${id}`,
          thread: { name: opts.thread ?? `${space}/threads/t${id}` },
          _pending: undefined,
        };
        delete message._pending;
        store.get(space)!.push(message);
        emit({ type: "outbox", clientId, status: "sent", message });
        emit(event(space, "message.v1.created", { message }));
        const other = (MEMBERS[space] ?? []).find((u) => u !== ME);
        if (space.startsWith("spaces/DM") && other) later(2500, () => emit({ type: "receipt", space, message: message.name, user: other, at: new Date().toISOString() }));
        if (space === "spaces/AAAgeneral") later(3000, () => {
          const r: Reaction = { name: `${message.name}/reactions/r${++seq}`, user: USERS["users/1002"], emoji: { unicode: "👍" } };
          message.emojiReactionSummaries = [...(message.emojiReactionSummaries ?? []), { emoji: { unicode: "👍" }, reactionCount: 1 }];
          emit(event(space, "reaction.v1.created", { reaction: r }));
        });
      });
      return optimistic;
    },
    async editMessage(name, text) {
      await delay();
      const f = findMessage(name);
      if (!f) throw new Error("404 not found");
      const m = { ...f.list[f.i]!, text, lastUpdateTime: new Date().toISOString() };
      f.list[f.i] = m;
      later(100, () => emit(event(name.split("/").slice(0, 2).join("/"), "message.v1.updated", { message: m })));
      return m;
    },
    async deleteMessage(name) {
      await delay();
      const f = findMessage(name);
      // Like Google with showDeleted: the row stays, the content goes, the deletion is described.
      const gone: Message = { name, deleteTime: new Date().toISOString(), deletionMetadata: { deletionType: f?.list[f.i]?.sender?.name === USERS[ME]!.name ? "CREATOR" : "SPACE_OWNER" } };
      if (f) f.list[f.i] = { ...gone, sender: f.list[f.i]!.sender, createTime: f.list[f.i]!.createTime, thread: f.list[f.i]!.thread, threadReply: f.list[f.i]!.threadReply };
      later(100, () => emit(event(name.split("/").slice(0, 2).join("/"), "message.v1.deleted", { message: gone })));
    },
    async toggleReaction(messageName, unicode) {
      await delay(80);
      const f = findMessage(messageName);
      if (!f) return;
      const m = f.list[f.i]!;
      const set = mine.get(messageName) ?? new Set<string>();
      mine.set(messageName, set);
      const space = messageName.split("/").slice(0, 2).join("/");
      const sums = [...(m.emojiReactionSummaries ?? [])];
      const i = sums.findIndex((s) => s.emoji?.unicode === unicode);
      const r: Reaction = { name: `${messageName}/reactions/${encodeURIComponent(unicode)}-me`, user: USERS[ME], emoji: { unicode } };
      if (set.has(unicode)) {
        set.delete(unicode);
        if (i >= 0) { const c = (sums[i]!.reactionCount ?? 1) - 1; if (c <= 0) sums.splice(i, 1); else sums[i] = { ...sums[i]!, reactionCount: c }; }
        m.emojiReactionSummaries = sums;
        later(60, () => emit(event(space, "reaction.v1.deleted", { reaction: r })));
      } else {
        set.add(unicode);
        if (i >= 0) sums[i] = { ...sums[i]!, reactionCount: (sums[i]!.reactionCount ?? 0) + 1 }; else sums.push({ emoji: { unicode }, reactionCount: 1 });
        m.emojiReactionSummaries = sums;
        later(60, () => emit(event(space, "reaction.v1.created", { reaction: r })));
      }
    },
    async markRead() { /* nothing to persist */ },
    async search(query) {
      await delay(150);
      const q = query.toLowerCase();
      const hits: Message[] = [];
      for (const list of store.values()) for (const m of list) if (m.text?.toLowerCase().includes(q)) hits.push(m);
      return hits.sort((a, b) => (a.createTime! < b.createTime! ? 1 : -1)).slice(0, 50);
    },
    async searchPeople(query) {
      await delay();
      const q = query.trim().toLowerCase();
      if (!q) return [];
      return [...Object.values(USERS), ...DIRECTORY].filter((u) => u.type !== "BOT" && ((u.displayName ?? "").toLowerCase().includes(q) || (u.email ?? "").toLowerCase().includes(q)));
    },
    async setupDm(user) {
      await delay();
      const have = SPACES.find((s) => s.spaceType === "DIRECT_MESSAGE" && MEMBERS[s.name]?.includes(user));
      if (have) return { ...have };
      const sp: Space = { name: `spaces/DM${user.split("/")[1]}`, spaceType: "DIRECT_MESSAGE", membershipCount: { joinedDirectHumanUserCount: 2 } };
      SPACES.push(sp);
      MEMBERS[sp.name] = [ME, user];
      store.set(sp.name, []);
      return { ...sp };
    },
    async setupGroup(members) {
      await delay();
      if (members.length < 2) throw new Error("INVALID_ARGUMENT: a group chat needs at least two other members");
      const sp: Space = { name: `spaces/GRP${++seq}`, spaceType: "GROUP_CHAT", membershipCount: { joinedDirectHumanUserCount: members.length + 1 } };
      SPACES.push(sp);
      MEMBERS[sp.name] = [ME, ...members];
      store.set(sp.name, []);
      return { ...sp };
    },
    async addMembers(space, users) {
      await delay();
      const list = (MEMBERS[space] ??= []);
      const out: Membership[] = [];
      for (const u of users) {
        if (!list.includes(u)) list.push(u);
        const m: Membership = { name: `${space}/members/${u.split("/")[1]}`, state: "JOINED", role: "ROLE_MEMBER", member: USERS[u] ?? DIRECTORY.find((d) => d.name === u) };
        out.push(m);
        later(150, () => emit(event(space, "membership.v1.created", { membership: m })));
      }
      return out;
    },
    async leaveSpace(space) {
      await delay();
      MEMBERS[space] = (MEMBERS[space] ?? []).filter((u) => u !== ME);
    },
    async createSpace(displayName, members) {
      await delay();
      if (SPACES.some((s) => s.displayName === displayName)) throw new Error("ALREADY_EXISTS: a space with that name already exists");
      const sp: Space = { name: `spaces/AAA${++seq}`, spaceType: "SPACE", displayName, spaceThreadingState: "THREADED_MESSAGES", membershipCount: { joinedDirectHumanUserCount: members.length + 1 } };
      SPACES.push(sp);
      MEMBERS[sp.name] = [ME, ...members];
      store.set(sp.name, []);
      return { ...sp };
    },
    async uploadAttachment(_space, file): Promise<Attachment> {
      await delay(300);
      const url = URL.createObjectURL(file);
      return { attachmentDataRef: { resourceName: `mock/${file.name}` }, contentName: file.name, contentType: file.type, downloadUri: url, thumbnailUri: url, source: "UPLOADED_CONTENT" };
    },
    async getAttachment(name) {
      await delay(250);
      const a = drive.get(name);
      if (!a) throw new Error("404 not found");
      return { ...a };
    },
    // Stands in for the shell's Meet mini window: records what is open so the flow can be checked.
    async raiseWindow() { (window as unknown as { __raised?: number }).__raised = Date.now(); },
    async openHuddleWindow(url, title) { (window as unknown as { __huddleWin?: unknown }).__huddleWin = { url, title }; },
    async closeHuddleWindow() { (window as unknown as { __huddleWin?: unknown }).__huddleWin = null; },
    async openUrl(url) {
      window.open(url, "_blank", "noopener,noreferrer");
    },
    async listTasks() { await delay(); return tasks.map((t) => ({ ...t })); },
    async addTask(title, notes, due) {
      await delay();
      const t: Task = { id: `task-${++taskSeq}`, title, status: "needsAction", updated: new Date().toISOString(), webViewLink: "https://tasks.google.com/", ...(notes ? { notes } : {}), ...(due ? { due } : {}) };
      tasks.unshift(t);
      return { ...t };
    },
    async setTaskDone(id, done) {
      await delay();
      const t = tasks.find((x) => x.id === id);
      if (!t) throw new Error("404 task not found");
      t.status = done ? "completed" : "needsAction";
      t.updated = new Date().toISOString();
      return { ...t };
    },
    async deleteTask(id) {
      await delay();
      const i = tasks.findIndex((x) => x.id === id);
      if (i >= 0) tasks.splice(i, 1);
    },
    notify: webNotify,
    async accountsList() { await delay(); return ACCOUNTS.map((a) => ({ ...a, active: a.id === activeAccount })); },
    async accountSwitch(id) {
      await delay(200);
      const a = ACCOUNTS.find((x) => x.id === id);
      if (!a) throw new Error("no such account");
      activeAccount = id;
      a.unread = 0;
      later(50, () => emit({ type: "account", id }));
      return { ...a, active: true };
    },
    async accountAdd() {
      await delay(200);
      const a: Account = { id: `acct-${ACCOUNTS.length + 1}`, signedIn: false, active: true, unread: 0 };
      ACCOUNTS.push(a);
      activeAccount = a.id;
      later(50, () => emit({ type: "account", id: a.id }));
      return a;
    },
    async accountRemove(id) {
      await delay();
      const i = ACCOUNTS.findIndex((x) => x.id === id);
      if (i >= 0 && ACCOUNTS[i]!.id !== activeAccount) ACCOUNTS.splice(i, 1);
    },
    async availabilityGet() { await delay(); return { ...availability }; },
    async availabilityMark(state, ttlSeconds) {
      await delay();
      availability = { ...availability, state: state === "away" ? "AWAY" : state === "dnd" ? "DO_NOT_DISTURB" : "ACTIVE", doNotDisturbMetadata: state === "dnd" ? { expirationTime: new Date(Date.now() + (ttlSeconds ?? 3600) * 1000).toISOString() } : undefined };
      return { ...availability };
    },
    async availabilityStatus(text, emoji, ttlSeconds) {
      await delay();
      if (text && text.length > 64) throw new Error("400 custom status text is limited to 64 characters");
      availability = { ...availability, customStatus: text ? { text, emoji: { unicode: emoji || "💬" }, ...(ttlSeconds ? { expireTime: new Date(Date.now() + ttlSeconds * 1000).toISOString() } : {}) } : undefined };
      return { ...availability };
    },
    async calendarList() { await delay(); return CALENDARS.map((c) => ({ ...c })); },
    async calendarEvents(calendarId, o): Promise<EventsPage> {
      await delay(150);
      const all = [...calEvents.values()].filter((e) => e.calendarId === calendarId);
      let items: CalEvent[];
      if (o.syncToken) {
        const since = Number(o.syncToken);
        if (!Number.isFinite(since) || since < calClock - 50) throw new Error("sync_expired");
        items = all.filter((e) => e._clock >= since);
      } else {
        const min = o.timeMin ? new Date(o.timeMin).getTime() : -Infinity;
        const max = o.timeMax ? new Date(o.timeMax).getTime() : Infinity;
        items = all.filter((e) => e.status !== "cancelled" && new Date(e.end.dateTime!).getTime() > min && new Date(e.start.dateTime!).getTime() < max).sort((a, b) => a.start.dateTime!.localeCompare(b.start.dateTime!));
      }
      return { items: items.map((e) => { const { _clock, ...rest } = e as CalEvent & { _clock: number }; void _clock; return rest; }), nextSyncToken: String(calClock + 1) };
    },
    async calendarCreateEvent(calendarId, ev): Promise<CalEvent> {
      await delay(300);
      const id = `ev-${++evSeq}`;
      calClock += 1;
      const code = meetCode(++meetSeq);
      const e: CalEvent & { _clock: number } = {
        id, calendarId, status: "confirmed", summary: ev.summary, description: ev.description, location: ev.location, start: ev.start, end: ev.end,
        htmlLink: `https://calendar.google.com/event?eid=${id}`, created: new Date().toISOString(), updated: new Date().toISOString(),
        attendees: [{ email: "arun@example.test", displayName: "Arun J", self: true, organizer: true, responseStatus: "accepted" }, ...ev.attendees.map((email) => ({ email, displayName: [...Object.values(USERS), ...DIRECTORY].find((u) => u.email === email)?.displayName, responseStatus: "needsAction" }))],
        organizer: { email: "arun@example.test", self: true },
        ...(ev.addMeet ? { hangoutLink: `https://meet.google.com/${code}`, conferenceData: { entryPoints: [{ entryPointType: "video", uri: `https://meet.google.com/${code}` }] } } : {}),
        _clock: calClock,
      };
      calEvents.set(id, e);
      // Someone answers the invite a little later, so the realtime path has something to show.
      later(6000, () => { const cur = calEvents.get(id); const a = cur?.attendees?.find((x) => !x.self); if (cur && a) { a.responseStatus = "accepted"; cur.updated = new Date().toISOString(); cur._clock = ++calClock; } });
      const { _clock, ...out } = e;
      return out;
    },
    async calendarUpdateEvent(calendarId, eventId, ev): Promise<CalEvent> {
      await delay(250);
      const e = calEvents.get(eventId);
      if (!e) throw new Error("404 event not found");
      const code = meetCode(++meetSeq);
      Object.assign(e, { calendarId, summary: ev.summary, description: ev.description, location: ev.location, start: ev.start, end: ev.end, recurrence: ev.recurrence ?? [], updated: new Date().toISOString(), _clock: ++calClock });
      const keep = (e.attendees ?? []).filter((a) => a.self);
      e.attendees = [...keep, ...ev.attendees.filter((em) => !keep.some((a) => a.email === em)).map((email) => ({ email, displayName: [...Object.values(USERS), ...DIRECTORY].find((u) => u.email === email)?.displayName, responseStatus: "needsAction" }))];
      if (ev.addMeet && !e.hangoutLink) { e.hangoutLink = `https://meet.google.com/${code}`; e.conferenceData = { entryPoints: [{ entryPointType: "video", uri: e.hangoutLink }] }; }
      const { _clock, ...out } = e;
      void _clock;
      return out;
    },
    async calendarRespond(_calendarId, eventId, response): Promise<CalEvent> {
      await delay();
      const e = calEvents.get(eventId);
      if (!e) throw new Error("404 event not found");
      e.attendees = (e.attendees ?? []).map((a) => (a.self ? { ...a, responseStatus: response } : a));
      e.updated = new Date().toISOString();
      e._clock = ++calClock;
      const { _clock, ...out } = e;
      return out;
    },
    async calendarDeleteEvent(_calendarId, eventId) {
      await delay();
      const e = calEvents.get(eventId);
      if (e) { e.status = "cancelled"; e._clock = ++calClock; }
    },
    async notesLoad() { await delay(200); return notesJson; },
    ...driveApi,
    async notesSave(json) { await delay(150); notesJson = json; },
    async createMeet(): Promise<MeetSpace> {
      await delay();
      const code = meetCode(++meetSeq);
      meets.set(code, { at: Date.now() });
      return { name: `spaces/meet-${meetSeq}`, meetingUri: `https://meet.google.com/${code}`, meetingCode: code };
    },
    async meetStatus(code): Promise<MeetStatus> {
      await delay();
      const m = meets.get(code);
      if (!m || m.message?.annotations?.[0]?.richLinkMetadata?.meetSpaceLinkData?.huddleStatus === "ENDED") return { active: false, participants: [] };
      const since = new Date(m.at).toISOString();
      const participants = [{ user: ME, displayName: USERS[ME]!.displayName, since }];
      if (Date.now() - m.at > 3000) participants.push({ user: "users/1002", displayName: "Priya Nair", since: new Date(m.at + 3000).toISOString() });
      return { active: true, conferenceRecord: `conferenceRecords/${code}`, participants };
    },
    async postHuddle(space, meetingUri, meetingCode, _mentionAll): Promise<Message> {
      await delay();
      const id = ++seq;
      const message: Message = {
        name: `${space}/messages/srv${id}`, sender: USERS[ME], createTime: new Date().toISOString(), text: "",
        thread: { name: `${space}/threads/t${id}` },
        annotations: [{ type: "RICH_LINK", startIndex: 0, length: 0, richLinkMetadata: { uri: meetingUri, richLinkType: "MEET_SPACE", meetSpaceLinkData: { meetingCode, type: "HUDDLE", huddleStatus: "STARTED" } } }],
      };
      store.get(space)!.push(message);
      const rec = meets.get(meetingCode);
      if (rec) { rec.message = message; rec.space = space; }
      later(100, () => emit(event(space, "message.v1.created", { message })));
      // The huddle ends on its own after a while (Google marks the message ENDED), unless the strip leaves first.
      later(90_000, () => endMockHuddle(meetingCode));
      return message;
    },
    async unfurl(url): Promise<LinkPreview | null> {
      await delay(400);
      const u = new URL(url);
      if (u.hostname !== "example.test") return null;
      if (u.pathname.startsWith("/notes")) return { url, title: "Deploy notes, week 45", description: "Relay 0.4 to the shared box on Tuesday, twin seed refresh, nothing user-facing. Rollback is the previous tag.", site: "Example wiki", image: IMG };
      if (u.pathname.startsWith("/cropyield")) return { url, title: "Yield Estimation dashboard", description: "District level crop yield estimates with the accessibility fixes from this week.", site: "example.test" };
      return { url, title: `Page at ${u.pathname}`, site: "example.test" };
    },
    relaySend(msg: ClientMsg) {
      if (msg.type === "ping") later(50, () => emit({ type: "pong", t: msg.t }));
      if (msg.type === "huddle_join") {
        huddleUsers = [ME];
        emit({ type: "huddle", space: msg.space, users: [...huddleUsers], at: new Date().toISOString() });
        // Priya joins a moment later; my name sorts lower than hers, so my client makes the offer and she answers.
        later(1200, () => { if (huddleUsers.includes(ME)) { huddleUsers = [ME, "users/1002"]; emit({ type: "huddle", space: msg.space, users: [...huddleUsers], at: new Date().toISOString() }); } });
      }
      if (msg.type === "huddle_leave") {
        huddleUsers = [];
        fakePeer?.close();
        fakePeer = null;
        emit({ type: "huddle", space: msg.space, users: [], at: new Date().toISOString() });
      }
      if (msg.type === "huddle_signal") void answerAsPriya(msg.space, msg.payload as { kind: string; sdp?: string; candidate?: RTCIceCandidateInit | null });
    },
    onMessage(cb) {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
  };
  return bridge;
}

// ---- Drive: an in-memory My Drive, a few shared files and one shared drive ----
const DOC = "application/vnd.google-apps.document";
const SHEET = "application/vnd.google-apps.spreadsheet";
const SLIDES = "application/vnd.google-apps.presentation";

function mockDrive(): Pick<Bridge, "driveStatus" | "driveConnect" | "driveList" | "driveDrives" | "driveCreateFolder" | "driveUpload" | "driveUpdate" | "driveContentUrl"> {
  const me = USERS[ME]!;
  const priya = USERS["users/1002"]!;
  const owner = (u: User) => [{ displayName: u.displayName, emailAddress: u.email, me: u.name === ME }];
  const drives: SharedDrive[] = [{ id: "drv-1", name: "Team" }];
  const items: DriveItem[] = [];
  const bytes = new Map<string, string>(); // id -> data URL
  let seq = 0;
  let connected = false;
  const day = 86_400_000;
  const add = (name: string, mimeType: string, parent: string, o: { by?: User; starred?: boolean; data?: string; ago?: number; shared?: boolean } = {}): string => {
    const id = `mf${++seq}`;
    const inDrive = parent === "drv-1" || items.find((i) => i.id === parent)?.driveId;
    const by = o.by ?? me;
    items.push({
      id, name, mimeType, parents: [parent], starred: !!o.starred, trashed: false,
      modifiedTime: new Date(Date.now() - (o.ago ?? seq) * day / 3).toISOString(),
      ...(o.data ? { size: String(Math.round(o.data.length * 0.75)) } : {}),
      webViewLink: `https://drive.google.com/${mimeType === FOLDER ? "drive/folders" : "file/d"}/${id}`,
      owners: inDrive ? [] : owner(by), shared: !!o.shared || by.name !== ME, driveId: inDrive ? "drv-1" : undefined,
      capabilities: { canEdit: by.name === ME || !!inDrive, canRename: by.name === ME || !!inDrive, canTrash: by.name === ME || !!inDrive, canAddChildren: mimeType === FOLDER, canDownload: true },
    });
    if (o.data) bytes.set(id, o.data);
    return id;
  };
  const txt = (s: string, type = "text/plain") => `data:${type};base64,${btoa(s)}`;
  const projects = add("Projects", FOLDER, "root", { starred: true });
  const payments = add("Payments revamp", FOLDER, projects);
  add("Payments revamp spec", DOC, payments, { ago: 1 });
  add("Rollout checklist.md", "text/markdown", payments, { data: txt("# Rollout\n\n- [x] Feature flag\n- [ ] Canary 5%\n- [ ] Full rollout\n", "text/markdown"), ago: 2 });
  add("latency.csv", "text/csv", payments, { data: txt("day,p50_ms,p95_ms\nMon,82,210\nTue,79,198\nWed,91,260\n", "text/csv"), ago: 3 });
  add("Roadmap", SLIDES, projects, { ago: 4 });
  const receipts = add("Receipts", FOLDER, "root");
  add("Laptop invoice.pdf", "application/pdf", receipts, { data: PDF, ago: 9 });
  const design = add("Design", FOLDER, "root");
  add("Glass panels.png", "image/svg+xml", design, { data: IMG, ago: 5 });
  add("motion-spec.pdf", "application/pdf", design, { data: PDF, ago: 6, shared: true });
  add("Budget 2026", SHEET, "root", { starred: true, ago: 2 });
  add("Meeting notes.txt", "text/plain", "root", { data: txt("Standup: relay reconnect fix shipped, twin seeds Drive now.\n"), ago: 1 });
  add("Q4 planning", DOC, "root", { by: priya, ago: 1 });
  const brand = add("Brand assets", FOLDER, "root", { by: priya, ago: 7 });
  add("Logo usage.pdf", "application/pdf", brand, { by: priya, data: PDF, ago: 8 });
  const eng = add("Engineering", FOLDER, "drv-1");
  add("On-call runbook", DOC, eng, { ago: 3 });
  add("Marketing", FOLDER, "drv-1");
  add("Team directory", SHEET, "drv-1", { ago: 10 });

  const visible = (where: DriveWhere): DriveItem[] => {
    const live = items.filter((i) => !i.trashed);
    const theirs = (i: DriveItem) => i.owners?.[0] && !i.owners[0].me;
    switch (where.kind) {
      case "root": return live.filter((i) => i.parents?.includes("root") && !theirs(i));
      case "folder": return live.filter((i) => i.parents?.includes(where.id));
      case "shared": return live.filter(theirs);
      case "starred": return live.filter((i) => i.starred);
      case "recent": return live.filter((i) => i.mimeType !== FOLDER).sort((a, b) => (b.modifiedTime ?? "").localeCompare(a.modifiedTime ?? ""));
      case "trash": return items.filter((i) => i.trashed);
      case "search": return live.filter((i) => i.name.toLowerCase().includes(where.query.toLowerCase()));
      case "drives": return [];
    }
  };
  const byFolderName = (a: DriveItem, b: DriveItem) => Number(b.mimeType === FOLDER) - Number(a.mimeType === FOLDER) || a.name.localeCompare(b.name);
  const wait = (ms = 140) => new Promise<void>((r) => setTimeout(r, ms));
  const find = (id: string) => {
    const f = items.find((i) => i.id === id);
    if (!f) throw new Error(`File not found: ${id}.`);
    return f;
  };
  return {
    async driveStatus() { await wait(60); return connected; },
    async driveConnect() { await wait(700); connected = true; return true; },
    async driveList(where) {
      await wait();
      const list = visible(where);
      return { files: (where.kind === "recent" || where.kind === "trash" ? list : [...list].sort(byFolderName)).map((f) => ({ ...f })) };
    },
    async driveDrives() { await wait(); return drives; },
    async driveCreateFolder(name, parent) { await wait(); return { ...find(add(name, FOLDER, parent, { ago: 0 })) }; },
    async driveUpload(file, parent) {
      const data = await new Promise<string>((res) => { const r = new FileReader(); r.onload = () => res(String(r.result)); r.readAsDataURL(file); });
      await wait(300);
      return { ...find(add(file.name, file.type || "application/octet-stream", parent, { data, ago: 0 })) };
    },
    async driveUpdate(id, patch, move) {
      await wait();
      const f = find(id);
      Object.assign(f, Object.fromEntries(Object.entries(patch).filter(([, v]) => v !== undefined)));
      if (move) {
        f.parents = [...(f.parents ?? []).filter((p) => p !== move.remove), move.add];
        f.driveId = move.add === "drv-1" || items.find((i) => i.id === move.add)?.driveId ? "drv-1" : undefined;
      }
      f.modifiedTime = new Date().toISOString();
      return { ...f };
    },
    async driveContentUrl(item) {
      await wait(60);
      return isGoogleFile(item.mimeType) ? PDF : bytes.get(item.id) ?? txt("");
    },
  };
}
