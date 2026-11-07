import { useEffect, useRef, useState } from "react";

/** True when the OS or Preferences asked for less motion; exit timers then collapse to zero. */
export function reducedMotion(): boolean {
  if (typeof document === "undefined") return true;
  if (document.documentElement.getAttribute("data-reduce-motion") === "true") return true;
  return typeof window.matchMedia === "function" && window.matchMedia("(prefers-reduced-motion: reduce)").matches;
}

/**
 * Keeps something mounted for `ms` after `show` turns false so its exit animation can play.
 * `exiting` is true during that window; render the `--out` modifier from it.
 */
export function useExit(show: boolean, ms = 160): { mounted: boolean; exiting: boolean } {
  const [phase, setPhase] = useState<"in" | "out" | "gone">(show ? "in" : "gone");
  useEffect(() => {
    if (show) { setPhase("in"); return; }
    const wait = reducedMotion() ? 0 : ms;
    setPhase((p) => (p === "in" && wait > 0 ? "out" : "gone"));
    const t = window.setTimeout(() => setPhase("gone"), wait);
    return () => window.clearTimeout(t);
  }, [show, ms]);
  return { mounted: phase !== "gone", exiting: phase === "out" };
}

/** True for `ms` after `dep` changes (not on mount): a one-shot class for pops and flashes. */
export function useFlash(dep: unknown, ms = 600): boolean {
  const [on, setOn] = useState(false);
  const first = useRef(true);
  useEffect(() => {
    if (first.current) { first.current = false; return; }
    if (reducedMotion()) return;
    setOn(true);
    const t = window.setTimeout(() => setOn(false), ms);
    return () => window.clearTimeout(t);
  }, [dep, ms]);
  return on;
}

/** The last defined value, so an element playing its exit keeps showing what it showed. */
export function useLast<T>(v: T | undefined | null): T | undefined {
  const ref = useRef<T | undefined>(v ?? undefined);
  if (v !== undefined && v !== null) ref.current = v;
  return ref.current;
}

/** Stagger index for list enter animations, capped so long lists do not wait on the tail. */
export const stagger = (i: number, cap = 12): React.CSSProperties => ({ ["--i" as string]: Math.min(i, cap) });
