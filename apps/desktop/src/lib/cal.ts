// Calendar date math and layout, pure so it is testable. Days are "YYYY-MM-DD" keys in local time.
import type { CalEvent } from "../bridge-types";

export const dayKey = (d: Date): string => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
export const fromKey = (k: string): Date => { const [y, m, d] = k.split("-").map(Number); return new Date(y!, (m ?? 1) - 1, d ?? 1); };
export const addDays = (d: Date, n: number): Date => { const x = new Date(d); x.setDate(x.getDate() + n); return x; };
export const startOfWeek = (d: Date, weekStartsOn = 1): Date => { const x = new Date(d); x.setHours(0, 0, 0, 0); const diff = (x.getDay() - weekStartsOn + 7) % 7; return addDays(x, -diff); };
export const startOfMonth = (d: Date): Date => new Date(d.getFullYear(), d.getMonth(), 1);
export const sameDayKey = (a: Date, b: Date): boolean => dayKey(a) === dayKey(b);

export const isAllDay = (e: CalEvent): boolean => !!e.start.date && !e.start.dateTime;
export const startOf = (e: CalEvent): Date => (e.start.dateTime ? new Date(e.start.dateTime) : fromKey(e.start.date ?? "1970-01-01"));
/** All-day ends are exclusive dates in Google's model; a one-day event ends the next morning. */
export const endOf = (e: CalEvent): Date => (e.end.dateTime ? new Date(e.end.dateTime) : fromKey(e.end.date ?? e.start.date ?? "1970-01-01"));
export const meetLinkOf = (e: CalEvent): string | undefined => e.hangoutLink ?? e.conferenceData?.entryPoints?.find((p) => p.entryPointType === "video")?.uri;
export const myResponse = (e: CalEvent): string | undefined => e.attendees?.find((a) => a.self)?.responseStatus;

/** Events that touch the day, all-day ones first, then by start. */
export function eventsOn(events: CalEvent[], day: Date): CalEvent[] {
  const s = new Date(day); s.setHours(0, 0, 0, 0);
  const e = addDays(s, 1);
  return events
    .filter((ev) => ev.status !== "cancelled" && startOf(ev) < e && endOf(ev) > s)
    .sort((a, b) => Number(isAllDay(b)) - Number(isAllDay(a)) || startOf(a).getTime() - startOf(b).getTime());
}

export interface Placed { ev: CalEvent; top: number; height: number; col: number; cols: number }
/** Side-by-side columns for overlapping timed events of one day; `top` and `height` are minutes from midnight. */
export function layoutDay(events: CalEvent[], day: Date): Placed[] {
  const s = new Date(day); s.setHours(0, 0, 0, 0);
  const timed = events.filter((e) => !isAllDay(e)).map((ev) => {
    const top = Math.max(0, (startOf(ev).getTime() - s.getTime()) / 60000);
    const bottom = Math.min(1440, (endOf(ev).getTime() - s.getTime()) / 60000);
    return { ev, top, height: Math.max(22, bottom - top), col: 0, cols: 1 };
  }).sort((a, b) => a.top - b.top || b.height - a.height);
  // Greedy column packing within clusters of overlapping events.
  let cluster: Placed[] = [];
  let clusterEnd = -1;
  const out: Placed[] = [];
  const flush = () => {
    const cols = cluster.reduce((m, p) => Math.max(m, p.col + 1), 1);
    for (const p of cluster) p.cols = cols;
    out.push(...cluster);
    cluster = [];
  };
  for (const p of timed) {
    if (cluster.length && p.top >= clusterEnd) flush();
    const taken = new Set(cluster.filter((q) => q.top + q.height > p.top).map((q) => q.col));
    let col = 0;
    while (taken.has(col)) col++;
    p.col = col;
    cluster.push(p);
    clusterEnd = Math.max(clusterEnd, p.top + p.height);
  }
  if (cluster.length) flush();
  return out;
}

export const fmtTime = (d: Date): string => d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
export const fmtRange = (e: CalEvent): string => (isAllDay(e) ? "All day" : `${fmtTime(startOf(e))} to ${fmtTime(endOf(e))}`);
export const fmtDayLong = (d: Date): string => d.toLocaleDateString([], { weekday: "long", month: "long", day: "numeric" });
export const MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/** The RRULE for the recurrence picker; "" means a single event. */
export function rrule(kind: "none" | "daily" | "weekdays" | "weekly" | "monthly" | "yearly", on: Date): string[] {
  const days = ["SU", "MO", "TU", "WE", "TH", "FR", "SA"];
  switch (kind) {
    case "daily": return ["RRULE:FREQ=DAILY"];
    case "weekdays": return ["RRULE:FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR"];
    case "weekly": return [`RRULE:FREQ=WEEKLY;BYDAY=${days[on.getDay()]}`];
    case "monthly": return [`RRULE:FREQ=MONTHLY;BYMONTHDAY=${on.getDate()}`];
    case "yearly": return ["RRULE:FREQ=YEARLY"];
    default: return [];
  }
}
export function describeRrule(lines: string[] | undefined): string {
  const r = lines?.find((l) => l.startsWith("RRULE:"));
  if (!r) return "";
  if (r.includes("FREQ=DAILY")) return "Every day";
  if (r.includes("BYDAY=MO,TU,WE,TH,FR")) return "Every weekday";
  if (r.includes("FREQ=WEEKLY")) return "Every week";
  if (r.includes("FREQ=MONTHLY")) return "Every month";
  if (r.includes("FREQ=YEARLY")) return "Every year";
  return "Repeats";
}

/** The next event that has not ended, for the top-bar chip; `withinMin` keeps it to the near future. */
export function nextUp(events: CalEvent[], now = new Date(), withinMin = 240): CalEvent | undefined {
  return events
    .filter((e) => e.status !== "cancelled" && !isAllDay(e) && myResponse(e) !== "declined" && endOf(e) > now && startOf(e).getTime() - now.getTime() < withinMin * 60000)
    .sort((a, b) => startOf(a).getTime() - startOf(b).getTime())[0];
}
