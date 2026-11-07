import { useMemo } from "react";
import type { Space, User } from "../bridge-types";
import { joinHuddle, leaveHuddle, leaveSpace, openExternal, openSpaceTasks, setPanel, startHuddle, startMeet, toast, toggleMute, toggleStar, toggleThreadPanel, ui } from "../actions";
import { liveHuddleIn } from "../lib/meet";
import { useExit, useFlash } from "../lib/motion";
import { dmPeer, spaceLabel, useStore } from "../store";
import { Avatar, Presence } from "./Avatar";
import { Icon } from "./icons";
import { Menu } from "./Rail";
import { isMeetChat } from "./Sidebar";

/** A stable hue per space, so each emblem keeps its colour across sessions. */
export function hueOf(name: string): number {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
  return h % 360;
}

/**
 * The space header: an emblem in the space's own colour, the name as a menu, what the space is about with member
 * and online counts, a facepile into the members panel, then Meet, Files, Tasks, Pinned and the usual tools.
 * A spectrum line sweeps along the bottom each time the space changes.
 */
export function Header({ space }: { space: Space }) {
  const members = useStore((s) => s.members[space.name]);
  const me = useStore((s) => s.auth.user?.name);
  const starred = useStore((s) => s.starred.includes(space.name));
  const muted = useStore((s) => s.muted.includes(space.name));
  const panel = useStore((s) => s.ui.panel);
  const hasThread = useStore((s) => !!s.ui.thread);
  const presence = useStore((s) => s.presence);
  const menuOpen = useStore((s) => s.ui.spaceMenu);
  const pins = useStore((s) => (s.pinned[space.name] ?? []).length);
  const voiceIn = useStore((s) => (s.huddles[space.name] ?? []).length);
  const mine = useStore((s) => s.huddle.space === space.name);
  const items = useStore((s) => s.messages[space.name]?.items);
  const live = useMemo(() => liveHuddleIn(items), [items]);
  const liveCount = useStore((s) => (s.huddle.space === space.name && s.huddle.kind === "meet" ? s.huddle.participants.length : 0));
  const starting = useStore((s) => !!s.huddle.starting && s.huddle.space === space.name);
  const peer = dmPeer(space, members, me);
  const peerPresence = peer ? presence[peer.name] : undefined;
  const label = spaceLabel(space, members, me);
  const people = useMemo(() => (members ?? []).map((m) => m.member).filter((u): u is User => !!u && u.name !== me && u.type !== "BOT"), [members, me]);
  const count = members?.length ?? space.membershipCount?.joinedDirectHumanUserCount ?? 0;
  const online = people.filter((u) => presence[u.name]?.state === "active").length;
  const isDM = space.spaceType === "DIRECT_MESSAGE";
  const isGroup = space.spaceType === "GROUP_CHAT";
  const starPop = useFlash(starred, 500);
  const menu = useExit(menuOpen, 120);
  const link = `https://mail.google.com/chat/u/0/#chat/space/${space.name.split("/")[1]}`;
  const toggle = (p: "members" | "files" | "pinned") => setPanel(panel === p ? "none" : p);
  const close = () => ui({ spaceMenu: false });
  const about = isDM
    ? peerPresence?.custom_status ? `${peerPresence.custom_status.emoji ?? ""} ${peerPresence.custom_status.text}`.trim() : peerPresence?.state === "active" ? "Active now" : peer?.email ?? "Direct message"
    : space.spaceDetails?.description ?? (isGroup ? "Group chat" : isMeetChat(space) ? "Chat from a meeting" : "No description yet");
  return (
    <div className="header glass-strong" style={{ ["--hue" as string]: hueOf(space.name) }}>
      <span key={space.name} className="header__sweep" aria-hidden="true" />
      <button type="button" className="header__emblem" onClick={() => setPanel(panel === "members" ? "none" : "members")} aria-label={`${label} details`} title={isDM ? "Details" : "Members"}>
        {isDM ? <Avatar user={peer} size={34} /> : <span className="header__glyph">{isGroup ? <Icon n="users" size={17} /> : isMeetChat(space) ? <Icon n="calendar" size={17} /> : "#"}</span>}
        {isDM && <Presence state={peerPresence?.state ?? "offline"} />}
      </button>
      <div className="header__id">
        <button type="button" className={`header__title${menuOpen ? " header__title--open" : ""}`} onClick={() => ui({ spaceMenu: !menuOpen })} aria-haspopup="menu" aria-expanded={menuOpen} aria-label={`${label} options`}>
          <span key={space.name} className="header__name-in">{label}</span>
          <Icon n="down" size={14} />
        </button>
        <button type="button" className={`iconbtn header__star${starred ? " header__star--on" : ""}${starPop ? " header__star--pop" : ""}`} aria-pressed={starred} aria-label={starred ? "Unstar" : "Star"} onClick={() => toggleStar(space.name)}>
          <Icon n="star" size={14} fill={starred} />
        </button>
        {muted && <span className="header__tag" title="Muted"><Icon n="belloff" size={11} />muted</span>}
        <div key={`${space.name}:${about}`} className="header__about" title={about}>
          <span className="header__desc">{about}</span>
          {!isDM && count > 0 && <span className="header__count">· {count} {count === 1 ? "member" : "members"}{online > 0 ? `, ${online} online` : ""}</span>}
        </div>
      </div>
      <div className="header__spacer" />
      {!isDM && people.length > 0 && (
        <button type="button" className="header__faces" onClick={() => toggle("members")} aria-pressed={panel === "members"} aria-label={`${count} members`} title="Members">
          {people.slice(0, 4).map((u) => <Avatar key={u.name} user={u} size={24} />)}
          {count > 4 && <span className="header__more">+{count - 4}</span>}
        </button>
      )}
      <div className="header__actions">
        <button
          type="button"
          className={`header__huddle${mine ? " header__huddle--on" : live || voiceIn ? " header__huddle--live" : ""}${starting ? " header__huddle--busy" : ""}`}
          disabled={starting}
          aria-busy={starting}
          onClick={() => (starting ? undefined : mine ? leaveHuddle() : live ? joinHuddle(space.name, live.link.code, live.link.uri, live.message.name) : void startHuddle(space.name))}
          title={mine ? "Leave the huddle" : live ? `Join the huddle ${live.message.sender?.displayName ?? "someone"} started` : "Start a huddle: a Meet everyone in the space can join"}
        >
          {starting ? <span className="spinner spinner--btn" /> : <Icon n="headphones" />}
          <span>{starting ? "Starting…" : mine ? (liveCount ? `In huddle · ${liveCount}` : "In huddle") : live ? "Join huddle" : voiceIn ? `Voice · ${voiceIn}` : "Huddle"}</span>
        </button>
        <button type="button" className="header__meet" onClick={() => void startMeet(space.name)} title="Start a video call: a Meet whose link is posted here for everyone"><Icon n="video" /><span>Meet</span></button>
        <span className="header__gap" />
        <button type="button" className={`iconbtn${panel === "files" ? " iconbtn--on" : ""}`} aria-pressed={panel === "files"} aria-label="Files" title="Files shared here" onClick={() => toggle("files")}><Icon n="folder" /></button>
        <button type="button" className="iconbtn" aria-label="Tasks from this space" title="Tasks from this space" onClick={() => openSpaceTasks(space.name)}><Icon n="check" /></button>
        <button type="button" className={`iconbtn header__pin${panel === "pinned" ? " iconbtn--on" : ""}`} aria-pressed={panel === "pinned"} aria-label={`Pinned, ${pins}`} title="Pinned messages" onClick={() => toggle("pinned")}><Icon n="pin" />{pins > 0 && <span className="header__pins">{pins}</span>}</button>
        <span className="header__gap" />
        <button type="button" className={`iconbtn${muted ? " iconbtn--on" : ""}`} aria-pressed={muted} aria-label={muted ? "Unmute" : "Mute"} title={muted ? "Muted: no badge, no sound" : "Mute"} onClick={() => toggleMute(space.name)}><Icon n={muted ? "belloff" : "bell"} /></button>
        <button type="button" className="iconbtn" aria-label="Search (⌘F)" title="Search ⌘F" onClick={() => ui({ focusSearch: Date.now() })}><Icon n="search" /></button>
        <button type="button" className={`iconbtn${panel === "thread" ? " iconbtn--on" : ""}`} aria-label="Toggle thread panel (⌘.)" title="Thread ⌘." disabled={!hasThread && panel !== "thread"} onClick={toggleThreadPanel}><Icon n="thread" /></button>
      </div>
      {menu.mounted && (
        <Menu className="menu--space" label={`${label} options`} exiting={menu.exiting} onClose={close}>
          <div className="menu__who">
            {isDM ? <Avatar user={peer} size={36} /> : <span className="menu__hash" style={{ ["--hue" as string]: hueOf(space.name) }}>{isGroup ? <Icon n="users" /> : "#"}</span>}
            <div><b>{label}</b><span>{isDM ? peer?.email ?? "Direct message" : space.spaceDetails?.description ?? `${count} members`}</span></div>
          </div>
          <button type="button" role="menuitem" className="menu__item" onClick={() => { close(); setPanel("members"); }}><Icon n="users" />{isDM ? "Details" : `Members, ${count}`}</button>
          {!isDM && <button type="button" role="menuitem" className="menu__item" onClick={() => ui({ spaceMenu: false, addMembersOpen: true })}><Icon n="plus" />Add members</button>}
          <button type="button" role="menuitem" className="menu__item" onClick={() => { close(); setPanel("files"); }}><Icon n="folder" />Files</button>
          <button type="button" role="menuitem" className="menu__item" onClick={() => { close(); setPanel("pinned"); }}><Icon n="pin" />Pinned{pins > 0 && <span className="kbd">{pins}</span>}</button>
          <button type="button" role="menuitem" className="menu__item" onClick={() => { close(); openSpaceTasks(space.name); }}><Icon n="check" />Tasks from here</button>
          <button type="button" role="menuitem" className="menu__item" onClick={() => { close(); void startMeet(space.name); }}><Icon n="video" />Start a video call</button>
          <div className="menu__sep" />
          <button type="button" role="menuitem" className="menu__item" onClick={() => { close(); toggleStar(space.name); }}><Icon n="star" fill={starred} />{starred ? "Unstar" : "Star"}</button>
          <button type="button" role="menuitem" className="menu__item" onClick={() => { close(); toggleMute(space.name); }}><Icon n={muted ? "bell" : "belloff"} />{muted ? "Unmute" : "Mute"}</button>
          <div className="menu__sep" />
          <button type="button" role="menuitem" className="menu__item" onClick={() => { close(); void navigator.clipboard?.writeText(link); toast("Link copied"); }}><Icon n="link" />Copy link</button>
          <button type="button" role="menuitem" className="menu__item" onClick={() => { close(); void openExternal(link); }}><Icon n="external" />Open in Google Chat</button>
          {!isDM && (
            <>
              <div className="menu__sep" />
              <button type="button" role="menuitem" className="menu__item menu__item--danger" onClick={() => { if (window.confirm(`Leave ${space.spaceType === "SPACE" ? `#${label}` : label}? You will need an invite to come back.`)) void leaveSpace(space.name).catch((e: unknown) => toast(`Could not leave: ${String(e)}`)); }}><Icon n="logout" />Leave space</button>
            </>
          )}
        </Menu>
      )}
    </div>
  );
}
