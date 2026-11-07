import { useMemo } from "react";
import type { Message, User } from "../bridge-types";
import { answerRing, dismissRing, huddleToBrowser, joinHuddle, leaveHuddle, openHuddleCall, openSpace, startMeet, toggleHuddleMute } from "../actions";
import { huddleOf } from "../lib/meet";
import { useExit } from "../lib/motion";
import { relative, parseTime } from "../lib/time";
import { nameMap, spaceLabel, useStore } from "../store";
import { Avatar } from "./Avatar";
import { Icon } from "./icons";

const NONE: string[] = [];

/**
 * The strip at the foot of the sidebar while in a huddle. Meet-backed: the call runs in its mini window and the
 * strip shows who is in it from the Meet API, with Open and Leave. Confab voice: mute, speaking rings, Leave.
 */
export function HuddleBar() {
  const h = useStore((s) => s.huddle);
  // Selectors must hand back the same reference while nothing changed, or the store loops.
  const voiceUsers = useStore((s) => (s.huddle.kind === "voice" && s.huddle.space ? s.huddles[s.huddle.space] ?? NONE : NONE));
  const sp = useStore((s) => s.spaces.find((x) => x.name === s.huddle.space));
  const members = useStore((s) => (s.huddle.space ? s.members[s.huddle.space] : undefined));
  const me = useStore((s) => s.auth.user);
  const { mounted, exiting } = useExit(!!h.kind, 200);
  const names = useMemo(() => nameMap(members), [members]);
  if (!mounted || !sp || !h.kind) return null;
  const label = spaceLabel(sp, members, me?.name);
  const meet = h.kind === "meet";
  const people: User[] = meet
    ? h.participants.map((p) => (p.user ? names[p.user] : undefined) ?? (p.user === me?.name ? me! : { name: p.user ?? p.displayName ?? "?", displayName: p.displayName }))
    : voiceUsers.map((u) => (u === me?.name ? me! : names[u] ?? { name: u }));
  const others = voiceUsers.filter((u) => u !== me?.name);
  const connected = others.filter((u) => h.peers[u] === "connected").length;
  const sub = meet
    ? h.starting ? "Starting the Meet…" : h.error ? "Meet did not answer" : !h.active ? "Joining in the Meet window…" : h.participants.length <= 1 ? "Waiting for others" : `${h.participants.length} in the call`
    : others.length === 0 ? "Waiting for others" : connected === others.length ? `${voiceUsers.length} in` : `Connecting ${connected}/${others.length}`;
  return (
    <div className={`huddle${exiting ? " huddle--out" : ""}`} role="region" aria-label="Huddle">
      <div className="huddle__head">
        <span className="huddle__dot" />
        <button type="button" className="huddle__space" onClick={() => openSpace(sp.name)} title="Go to the space">{sp.spaceType === "SPACE" ? `#${label}` : label}</button>
        <span className="huddle__sub">{sub}</span>
      </div>
      <div className="huddle__people">
        {people.map((u) => (
          <span key={u.name} className={`huddle__person${h.speaking.includes(u.name) ? " huddle__person--talking" : ""}${!meet && u.name !== me?.name && h.peers[u.name] === "failed" ? " huddle__person--failed" : ""}`} title={u.displayName ?? u.name}>
            <Avatar user={u} size={30} />
          </span>
        ))}
        {people.length === 0 && <span className="huddle__none">Nobody yet</span>}
      </div>
      <div className="huddle__bar">
        {meet ? (
          <>
            <button type="button" className="huddle__btn" aria-label="Show the call" title="Show the call window" disabled={!h.uri} onClick={() => h.uri && void openHuddleCall(h.uri, h.space)}><Icon n="video" /></button>
            <button type="button" className="huddle__btn" aria-label="Move the call to the browser" title="Move to the browser, to share your screen" disabled={!h.uri} onClick={huddleToBrowser}><Icon n="external" /></button>
            <span className="huddle__kind">Google Meet</span>
          </>
        ) : (
          <>
            <button type="button" className={`huddle__btn${h.muted ? " huddle__btn--muted" : ""}`} aria-pressed={h.muted} aria-label={h.muted ? "Unmute" : "Mute"} title={h.listenOnly ? "No microphone" : h.muted ? "Unmute" : "Mute"} disabled={h.listenOnly} onClick={toggleHuddleMute}>
              <Icon n={h.muted || h.listenOnly ? "micoff" : "mic"} />
            </button>
            <button type="button" className="huddle__btn" aria-label="Switch to Google Meet" title="Switch to Google Meet for people who are not on Confab" onClick={() => void startMeet(h.space)}><Icon n="video" /></button>
            <span className="huddle__kind">Confab voice</span>
          </>
        )}
        <span className="huddle__spacer" />
        <button type="button" className="huddle__leave" onClick={leaveHuddle}>Leave</button>
      </div>
      {!meet && h.listenOnly && <div className="huddle__note">Listening only: the microphone was not available.</div>}
    </div>
  );
}

/** Google's huddle message, drawn as its card: who started it, live or ended, and Join while it runs. */
export function HuddleCard({ m }: { m: Message }) {
  const link = huddleOf(m)!;
  const mine = useStore((s) => s.huddle.kind === "meet" && s.huddle.code === link.code);
  const count = useStore((s) => (s.huddle.kind === "meet" && s.huddle.code === link.code ? s.huddle.participants.length : 0));
  const space = m.name.split("/").slice(0, 2).join("/");
  const live = link.status === "STARTED";
  const title = !link.huddle ? "Google Meet" : live ? "Started a huddle" : link.status === "MISSED" ? "Missed huddle" : "Huddle ended";
  const when = link.status === "ENDED" && m.lastUpdateTime ? `ended ${relative(parseTime(m.lastUpdateTime))}` : link.uri.replace(/^https?:\/\//, "");
  return (
    <div className={`hcard${live || !link.huddle ? " hcard--live" : ""}`} role="group" aria-label={title}>
      <span className="hcard__icon"><Icon n={link.huddle ? "headphones" : "video"} /></span>
      <span className="hcard__body">
        <b>{title}</b>
        <span className="hcard__sub">{mine && count ? `${count} in the call · ` : ""}{when}</span>
      </span>
      {mine ? (
        <button type="button" className="btn" onClick={leaveHuddle}>Leave</button>
      ) : live || !link.huddle ? (
        <button type="button" className="btn btn--primary" onClick={() => joinHuddle(space, link.code, link.uri, m.name)}>Join</button>
      ) : null}
    </div>
  );
}

/** An incoming huddle, the way a call rings: who, where, Join or Decline. It stops by itself after a while. */
export function IncomingHuddle() {
  const r = useStore((s) => s.ringing);
  const sp = useStore((s) => s.spaces.find((x) => x.name === s.ringing?.space));
  const members = useStore((s) => (s.ringing ? s.members[s.ringing.space] : undefined));
  const me = useStore((s) => s.auth.user?.name);
  if (!r) return null;
  const where = sp && sp.spaceType !== "DIRECT_MESSAGE" ? `in ${sp.spaceType === "SPACE" ? "#" : ""}${spaceLabel(sp, members, me)}` : "Huddle";
  return (
    <div className="ring" role="alertdialog" aria-label={`${r.from} started a huddle`}>
      <span className="ring__icon"><Icon n="headphones" /></span>
      <span className="ring__body"><b>{r.from}</b><span>Started a huddle · {where}</span></span>
      <button type="button" className="btn ring__no" onClick={dismissRing}>Decline</button>
      <button type="button" className="btn btn--primary" autoFocus onClick={answerRing}>Join</button>
    </div>
  );
}
