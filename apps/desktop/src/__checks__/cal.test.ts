import { describe, expect, it } from "vitest";
import type { CalEvent } from "../bridge-types";
import { describeRrule, eventsOn, layoutDay, nextUp, rrule, startOfWeek } from "../lib/cal";

const ev = (id: string, start: string, end: string, extra: Partial<CalEvent> = {}): CalEvent => ({ id, start: { dateTime: start }, end: { dateTime: end }, ...extra });

describe("calendar math", () => {
  it("packs overlapping events into columns and keeps the rest full width", () => {
    const day = new Date(2026, 10, 7);
    const list = [ev("a", "2026-11-07T10:00:00", "2026-11-07T11:00:00"), ev("b", "2026-11-07T10:30:00", "2026-11-07T11:30:00"), ev("c", "2026-11-07T14:00:00", "2026-11-07T15:00:00")];
    const placed = layoutDay(eventsOn(list, day), day);
    const by = Object.fromEntries(placed.map((p) => [p.ev.id, p]));
    expect([by.a!.col, by.a!.cols, by.b!.col, by.b!.cols]).toEqual([0, 2, 1, 2]);
    expect([by.c!.col, by.c!.cols, by.c!.top, by.c!.height]).toEqual([0, 1, 14 * 60, 60]);
  });
  it("lists all-day events first and skips cancelled ones", () => {
    const day = new Date(2026, 10, 7);
    const list = [ev("t", "2026-11-07T09:00:00", "2026-11-07T09:30:00"), { id: "ad", start: { date: "2026-11-07" }, end: { date: "2026-11-08" } } as CalEvent, ev("x", "2026-11-07T12:00:00", "2026-11-07T13:00:00", { status: "cancelled" })];
    expect(eventsOn(list, day).map((e) => e.id)).toEqual(["ad", "t"]);
  });
  it("weeks start on Monday and recurrence rules read back", () => {
    expect(startOfWeek(new Date(2026, 10, 7)).getDay()).toBe(1);
    expect(rrule("weekly", new Date(2026, 10, 7))).toEqual(["RRULE:FREQ=WEEKLY;BYDAY=SA"]);
    expect(describeRrule(rrule("weekdays", new Date()))).toBe("Every weekday");
    expect(describeRrule(undefined)).toBe("");
  });
  it("picks the next event that has not ended, ignoring declined ones", () => {
    const now = new Date(2026, 10, 7, 10, 5);
    const list = [ev("past", "2026-11-07T08:00:00", "2026-11-07T09:00:00"), ev("now", "2026-11-07T10:00:00", "2026-11-07T10:30:00"), ev("declined", "2026-11-07T09:50:00", "2026-11-07T10:40:00", { attendees: [{ email: "me", self: true, responseStatus: "declined" }] }), ev("later", "2026-11-07T11:00:00", "2026-11-07T12:00:00")];
    expect(nextUp(list, now)?.id).toBe("now");
  });
});
