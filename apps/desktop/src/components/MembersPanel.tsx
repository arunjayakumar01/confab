import type { Space } from "../bridge-types";
import { setPanel } from "../actions";
import { stagger } from "../lib/motion";
import { spaceLabel, useStore } from "../store";
import { Avatar, Presence } from "./Avatar";
import { Icon } from "./icons";

export function MembersPanel({ space, exiting }: { space: Space; exiting?: boolean }) {
  const members = useStore((s) => s.members[space.name]);
  const presence = useStore((s) => s.presence);
  const me = useStore((s) => s.auth.user?.name);
  const myState = useStore((s) => s.me);
  const label = spaceLabel(space, members, me);
  const list = [...(members ?? [])].sort((a, b) => (a.member?.displayName ?? "").localeCompare(b.member?.displayName ?? ""));
  return (
    <aside className={`panel glass${exiting ? " panel--out" : ""}`} aria-label="Members">
      <div className="panel__head">
        <span className="panel__title">Members</span>
        <span className="panel__sub">{space.spaceType === "SPACE" ? `#${label}` : label}</span>
        <button type="button" className="iconbtn" aria-label="Close members" onClick={() => setPanel("none")}>
          <Icon n="x" />
        </button>
      </div>
      <div className="panel__scroll">
        {space.spaceDetails?.description && <div className="panel__count" style={{ fontSize: 13 }}>{space.spaceDetails.description}</div>}
        <div className="panel__count">{list.length} {list.length === 1 ? "member" : "members"}</div>
        {list.map((m, i) => {
          const u = m.member;
          const isMe = u?.name === me;
          const p = isMe ? { state: myState.state, custom_status: myState.custom } : u ? presence[u.name] : undefined;
          return (
            <div key={m.name} className="member" style={stagger(i, 16)}>
              <Avatar user={u} size={28} />
              <div>
                <div className="member__name">
                  {u?.displayName ?? u?.name ?? "Unknown"}
                  <Presence state={p?.state ?? "offline"} />
                  {isMe && <span className="member__you">you</span>}
                </div>
                {(p?.custom_status || u?.email) && (
                  <div className="member__status">{p?.custom_status ? `${p.custom_status.emoji ?? ""} ${p.custom_status.text}`.trim() : u?.email}</div>
                )}
              </div>
            </div>
          );
        })}
        {!members && <div className="list__top">Loading…</div>}
      </div>
    </aside>
  );
}
