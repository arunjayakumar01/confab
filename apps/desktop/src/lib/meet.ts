import type { Message } from "../bridge-types";

export interface HuddleLink { uri: string; code: string; status: "STARTED" | "ENDED" | "MISSED" | "LINK"; huddle: boolean }

const MEET_RE = /https?:\/\/meet\.google\.com\/([a-z]{3}-[a-z]{4}-[a-z]{3})\b/i;

/**
 * The Meet behind a message: Google's huddle is a rich-link annotation (type HUDDLE with a status that moves to
 * ENDED when it is over); a plain Meet link in the text is a meeting without huddle state.
 */
export function huddleOf(m: Pick<Message, "annotations" | "text" | "formattedText">): HuddleLink | null {
  for (const a of m.annotations ?? []) {
    const r = a.richLinkMetadata;
    if (!r || r.richLinkType !== "MEET_SPACE" || !r.uri) continue;
    const d = r.meetSpaceLinkData;
    const code = d?.meetingCode ?? MEET_RE.exec(r.uri)?.[1] ?? "";
    const huddle = d?.type === "HUDDLE";
    const status = huddle ? ((d?.huddleStatus as HuddleLink["status"]) ?? "STARTED") : "LINK";
    return { uri: r.uri, code, status, huddle };
  }
  const hit = MEET_RE.exec(m.formattedText ?? m.text ?? "");
  if (hit) return { uri: hit[0], code: hit[1]!, status: "LINK", huddle: false };
  return null;
}

/** A huddle someone just started, by Google's annotation or by the text Confab posts when Google refuses it. */
export function isHuddleStart(m: Pick<Message, "annotations" | "text" | "formattedText">): boolean {
  const link = huddleOf(m);
  if (!link) return false;
  return link.huddle ? link.status === "STARTED" : /^Started a huddle\b/.test(m.text ?? m.formattedText ?? "");
}

const LIVE_MS = 6 * 3600_000;
/** The newest huddle in a space that is still marked started and not older than six hours: what the header offers to join. */
export function liveHuddleIn(items: Message[] | undefined, now = Date.now()): { message: Message; link: HuddleLink } | null {
  if (!items) return null;
  for (let i = items.length - 1; i >= 0; i--) {
    const m = items[i]!;
    const link = huddleOf(m);
    if (!link?.huddle) continue;
    if (link.status !== "STARTED") return null;
    return now - new Date(m.createTime ?? 0).getTime() < LIVE_MS ? { message: m, link } : null;
  }
  return null;
}
