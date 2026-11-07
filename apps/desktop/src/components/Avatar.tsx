import { useEffect, useRef, useState } from "react";
import type { PresenceState, User } from "../bridge-types";
import { useFlash } from "../lib/motion";

const HUES = [152, 205, 338, 38, 14, 262, 184, 96];
function hue(name: string): number {
  let h = 0;
  for (let i = 0; i < name.length; i++) h = (h * 31 + name.charCodeAt(i)) >>> 0;
  return HUES[h % HUES.length]!;
}
export function initials(u?: User): string {
  const n = (u?.displayName ?? u?.email ?? "?").trim();
  const parts = n.split(/\s+/).filter(Boolean);
  return ((parts[0]?.[0] ?? "?") + (parts.length > 1 ? parts[parts.length - 1]![0] ?? "" : "")).toUpperCase();
}

/** Initials on a stable per-user colour; the photo fades in over them once it has loaded. */
export function Avatar({ user, size = 36 }: { user?: User; size?: number }) {
  const name = user?.name ?? "?";
  const src = user?.avatarUrl;
  const [ok, setOk] = useState(false);
  const img = useRef<HTMLImageElement>(null);
  useEffect(() => {
    setOk(false);
    if (img.current?.complete && img.current.naturalWidth > 0) setOk(true);
  }, [src]);
  const style = { width: size, height: size, fontSize: size * 0.4, ["--av" as string]: `hsl(${hue(name)} 52% 38%)` };
  return (
    <span className="avatar" style={style} aria-hidden="true">
      {initials(user)}
      {src && <img ref={img} className={ok ? "avatar__ok" : undefined} src={src} alt="" referrerPolicy="no-referrer" onLoad={() => setOk(true)} />}
    </span>
  );
}

export function Presence({ state, title }: { state?: PresenceState; title?: string }) {
  const s = state ?? "offline";
  const flash = useFlash(s, 450);
  const label = s === "active" ? "Active" : s === "away" ? "Away" : s === "dnd" ? "Do not disturb" : "Offline";
  return <span className={`presence presence--${s}${flash ? " presence--flash" : ""}`} role="img" aria-label={title ?? label} title={title ?? label} />;
}
