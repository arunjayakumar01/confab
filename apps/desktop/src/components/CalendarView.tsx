import { useEffect, useMemo, useRef, useState, type FormEvent } from "react";
import type { CalEvent, EventInput, User } from "../bridge-types";
import { calStep, closeEventEditor, closeFeed, deleteEvent, editEvent, loadCalendar, openExternal, respondEvent, saveEvent, selectEvent, setCalDay, setCalView, toggleCalendar } from "../actions";
import { MONTHS, addDays, dayKey, describeRrule, endOf, eventsOn, fmtDayLong, fmtRange, fmtTime, fromKey, isAllDay, layoutDay, meetLinkOf, myResponse, rrule, startOf, startOfMonth, startOfWeek } from "../lib/cal";
import { useExit } from "../lib/motion";
import { todayKey, useStore, type CalView } from "../store";
import { Avatar } from "./Avatar";
import { Icon } from "./icons";
import { Modal } from "./Modal";
import { PeoplePicker, userLabel } from "./PeoplePicker";
import { Menu } from "./Rail";

const HOUR_PX = 56;
const DOW = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const calKey = (e: CalEvent) => `${e.calendarId ?? "primary"}:${e.id}`;

/** Google Calendar in the main pane: month, week, day and agenda, a popover per event, and the event editor. */
export function CalendarView() {
  const cal = useStore((s) => s.calendar);
  const visible = useMemo(() => Object.values(cal.events).filter((e) => !cal.hidden.includes(e.calendarId ?? "primary")), [cal.events, cal.hidden]);
  const colorOf = useMemo(() => { const m: Record<string, string> = {}; for (const c of cal.calendars) m[c.id] = c.backgroundColor ?? "#7986cb"; return m; }, [cal.calendars]);
  const day = fromKey(cal.day);
  const title = cal.view === "month" ? `${MONTHS[day.getMonth()]} ${day.getFullYear()}` : cal.view === "day" ? fmtDayLong(day) : cal.view === "agenda" ? "Agenda" : weekTitle(day);
  const short = cal.view === "month" ? `${MONTHS[day.getMonth()]!.slice(0, 3)} ${day.getFullYear()}` : cal.view === "day" ? `${MONTHS[day.getMonth()]!.slice(0, 3)} ${day.getDate()}` : cal.view === "agenda" ? "Agenda" : `${MONTHS[startOfWeek(day).getMonth()]!.slice(0, 3)} ${startOfWeek(day).getDate()} to ${addDays(startOfWeek(day), 6).getDate()}`;
  const editor = useExit(cal.editing !== undefined, 140);
  return (
    <>
      <div className="header glass-strong cal__head">
        <div className="header__name"><span key={title} className="header__name-in"><span className="cal__title-long">{title}</span><span className="cal__title-short">{short}</span></span></div>
        <div className="cal__nav">
          <button type="button" className="iconbtn" aria-label="Previous" onClick={() => calStep(-1)}><Icon n="left" /></button>
          <button type="button" className="btn cal__today" onClick={() => setCalDay(todayKey())}>Today</button>
          <button type="button" className="iconbtn" aria-label="Next" onClick={() => calStep(1)}><Icon n="right" /></button>
        </div>
        <div className="seg cal__views" role="radiogroup" aria-label="View">
          {(["day", "week", "month", "agenda"] as CalView[]).map((v) => (
            <button key={v} type="button" role="radio" aria-checked={cal.view === v} aria-pressed={cal.view === v} onClick={() => setCalView(v)}>{v[0]!.toUpperCase() + v.slice(1)}</button>
          ))}
        </div>
        <div className="header__spacer" />
        <span className="cal__sync" title={cal.polledAt ? `Synced ${new Date(cal.polledAt).toLocaleTimeString()}` : "Not synced yet"}>{cal.loading ? "Syncing…" : cal.error ? "Calendar did not answer" : "Live"}</span>
        <button type="button" className="btn btn--primary cal__new" onClick={() => editEvent(null)} title="New event"><Icon n="plus" /><span>New event</span></button>
        <button type="button" className="iconbtn" aria-label="Close" onClick={closeFeed}><Icon n="x" /></button>
      </div>
      <div className="cal">
        <aside className="cal__side">
          <MiniMonth day={day} events={visible} onPick={(d) => { setCalDay(dayKey(d)); if (cal.view === "month") setCalView("day"); }} />
          <div className="cal__cals">
            <div className="cal__label">Calendars</div>
            {cal.calendars.map((c) => (
              <label key={c.id} className="cal__cal">
                <input type="checkbox" checked={!cal.hidden.includes(c.id)} onChange={() => toggleCalendar(c.id)} />
                <span className="cal__swatch" style={{ background: c.backgroundColor ?? "#7986cb" }} />
                <span className="cal__calname">{c.summary}</span>
              </label>
            ))}
            {cal.calendars.length === 0 && !cal.loading && <div className="sidebar__empty">{cal.error ? "The Calendar API did not answer. It needs to be enabled in the project and the sign-in needs the calendar scopes." : "No calendars."}{cal.error && <button type="button" className="btn" onClick={() => void loadCalendar()}>Try again</button>}</div>}
          </div>
        </aside>
        <div className="cal__main">
          {cal.view === "month" && <MonthGrid day={day} events={visible} colorOf={colorOf} />}
          {cal.view === "week" && <TimeGrid days={Array.from({ length: 7 }, (_, i) => addDays(startOfWeek(day), i))} events={visible} colorOf={colorOf} />}
          {cal.view === "day" && <TimeGrid days={[day]} events={visible} colorOf={colorOf} />}
          {cal.view === "agenda" && <Agenda day={day} events={visible} colorOf={colorOf} />}
        </div>
      </div>
      {cal.selected && <EventPopover eventKey={cal.selected} />}
      {editor.mounted && <EventDialog exiting={editor.exiting} />}
    </>
  );
}

function weekTitle(d: Date): string {
  const a = startOfWeek(d);
  const b = addDays(a, 6);
  return a.getMonth() === b.getMonth() ? `${MONTHS[a.getMonth()]} ${a.getDate()} to ${b.getDate()}, ${a.getFullYear()}` : `${MONTHS[a.getMonth()]!.slice(0, 3)} ${a.getDate()} to ${MONTHS[b.getMonth()]!.slice(0, 3)} ${b.getDate()}, ${b.getFullYear()}`;
}

function MiniMonth({ day, events, onPick }: { day: Date; events: CalEvent[]; onPick: (d: Date) => void }) {
  const first = startOfMonth(day);
  const start = startOfWeek(first);
  const cells = Array.from({ length: 42 }, (_, i) => addDays(start, i));
  const busy = useMemo(() => new Set(events.filter((e) => e.status !== "cancelled").map((e) => dayKey(startOf(e)))), [events]);
  const today = todayKey();
  return (
    <div className="mini">
      <div className="mini__title">{MONTHS[first.getMonth()]} {first.getFullYear()}</div>
      <div className="mini__grid">
        {DOW.map((d) => <span key={d} className="mini__dow">{d[0]}</span>)}
        {cells.map((d) => {
          const k = dayKey(d);
          return (
            <button key={k} type="button" className={`mini__day${d.getMonth() !== first.getMonth() ? " mini__day--out" : ""}${k === today ? " mini__day--today" : ""}${k === dayKey(day) ? " mini__day--on" : ""}${busy.has(k) ? " mini__day--busy" : ""}`} onClick={() => onPick(d)}>{d.getDate()}</button>
          );
        })}
      </div>
    </div>
  );
}

function MonthGrid({ day, events, colorOf }: { day: Date; events: CalEvent[]; colorOf: Record<string, string> }) {
  const first = startOfMonth(day);
  const start = startOfWeek(first);
  const cells = Array.from({ length: 42 }, (_, i) => addDays(start, i));
  const today = todayKey();
  return (
    <div className="month">
      <div className="month__dows">{DOW.map((d) => <span key={d}>{d}</span>)}</div>
      <div className="month__grid">
        {cells.map((d) => {
          const k = dayKey(d);
          const list = eventsOn(events, d);
          return (
            <div key={k} className={`month__cell${d.getMonth() !== first.getMonth() ? " month__cell--out" : ""}${k === today ? " month__cell--today" : ""}`} onDoubleClick={() => editEvent({ start: { dateTime: new Date(d.getFullYear(), d.getMonth(), d.getDate(), 9).toISOString() } })}>
              <button type="button" className="month__num" onClick={() => { setCalDay(k); setCalView("day"); }}>{d.getDate()}</button>
              {list.slice(0, 3).map((e) => (
                <button key={calKey(e)} type="button" className={`month__ev${isAllDay(e) ? " month__ev--allday" : ""}${myResponse(e) === "declined" ? " month__ev--declined" : ""}`} style={{ ["--c" as string]: colorOf[e.calendarId ?? "primary"] ?? "#7986cb" }} onClick={() => selectEvent(calKey(e))}>
                  {!isAllDay(e) && <span className="month__time">{fmtTime(startOf(e))}</span>}{e.summary ?? "(No title)"}
                </button>
              ))}
              {list.length > 3 && <button type="button" className="month__more" onClick={() => { setCalDay(k); setCalView("day"); }}>{list.length - 3} more</button>}
            </div>
          );
        })}
      </div>
    </div>
  );
}

function TimeGrid({ days, events, colorOf }: { days: Date[]; events: CalEvent[]; colorOf: Record<string, string> }) {
  const scroller = useRef<HTMLDivElement>(null);
  const [now, setNow] = useState(new Date());
  useEffect(() => { const t = window.setInterval(() => setNow(new Date()), 60_000); return () => window.clearInterval(t); }, []);
  useEffect(() => { const el = scroller.current; if (el) el.scrollTop = Math.max(0, (now.getHours() - 1.5) * HOUR_PX); // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [days.map(dayKey).join()]);
  const today = todayKey();
  const nowTop = (now.getHours() * 60 + now.getMinutes()) * (HOUR_PX / 60);
  const onPickTime = (d: Date, e: React.MouseEvent<HTMLDivElement>) => {
    if ((e.target as HTMLElement).closest(".tg__ev")) return;
    const y = e.clientY - e.currentTarget.getBoundingClientRect().top + e.currentTarget.scrollTop;
    const mins = Math.floor(y / (HOUR_PX / 60) / 30) * 30;
    const start = new Date(d.getFullYear(), d.getMonth(), d.getDate(), 0, mins);
    editEvent({ start: { dateTime: start.toISOString() }, end: { dateTime: new Date(start.getTime() + 30 * 60000).toISOString() } });
  };
  return (
    <div className={`tg${days.length === 1 ? " tg--day" : ""}`}>
      <div className="tg__days">
        <span className="tg__gutter" />
        {days.map((d) => (
          <button key={dayKey(d)} type="button" className={`tg__dayhead${dayKey(d) === today ? " tg__dayhead--today" : ""}`} onClick={() => { setCalDay(dayKey(d)); setCalView("day"); }}>
            <span>{DOW[(d.getDay() + 6) % 7]}</span><b>{d.getDate()}</b>
          </button>
        ))}
      </div>
      <div className="tg__allday">
        <span className="tg__gutter">all day</span>
        {days.map((d) => (
          <div key={dayKey(d)} className="tg__alldaycell">
            {eventsOn(events, d).filter(isAllDay).map((e) => (
              <button key={calKey(e)} type="button" className="tg__chip" style={{ ["--c" as string]: colorOf[e.calendarId ?? "primary"] ?? "#7986cb" }} onClick={() => selectEvent(calKey(e))}>{e.summary ?? "(No title)"}</button>
            ))}
          </div>
        ))}
      </div>
      <div className="tg__scroll" ref={scroller}>
        <div className="tg__hours" style={{ height: 24 * HOUR_PX }}>
          {Array.from({ length: 24 }, (_, h) => <span key={h} className="tg__hour" style={{ top: h * HOUR_PX }}>{h === 0 ? "" : new Date(2000, 0, 1, h).toLocaleTimeString([], { hour: "numeric" })}</span>)}
        </div>
        {days.map((d) => (
          <div key={dayKey(d)} className={`tg__col${dayKey(d) === today ? " tg__col--today" : ""}`} style={{ height: 24 * HOUR_PX }} onDoubleClick={(e) => onPickTime(d, e)}>
            {Array.from({ length: 24 }, (_, h) => <span key={h} className="tg__line" style={{ top: h * HOUR_PX }} />)}
            {layoutDay(eventsOn(events, d), d).map((p) => {
              const mine = myResponse(p.ev);
              return (
                <button
                  key={calKey(p.ev)}
                  type="button"
                  className={`tg__ev${p.height * (HOUR_PX / 60) < 42 ? " tg__ev--short" : ""}${mine === "declined" ? " tg__ev--declined" : ""}${mine === "tentative" || mine === "needsAction" ? " tg__ev--maybe" : ""}`}
                  style={{ top: p.top * (HOUR_PX / 60), height: p.height * (HOUR_PX / 60) - 2, left: `calc(${(p.col / p.cols) * 100}% + 2px)`, width: `calc(${100 / p.cols}% - 4px)`, ["--c" as string]: colorOf[p.ev.calendarId ?? "primary"] ?? "#7986cb" }}
                  onClick={() => selectEvent(calKey(p.ev))}
                  title={`${p.ev.summary ?? "(No title)"} · ${fmtRange(p.ev)}`}
                >
                  <b>{p.ev.summary ?? "(No title)"}</b>
                  <span>{fmtTime(startOf(p.ev))}{meetLinkOf(p.ev) ? " · Meet" : ""}</span>
                </button>
              );
            })}
            {dayKey(d) === today && <span className="tg__now" style={{ top: nowTop }} />}
          </div>
        ))}
      </div>
    </div>
  );
}

function Agenda({ day, events, colorOf }: { day: Date; events: CalEvent[]; colorOf: Record<string, string> }) {
  const days = Array.from({ length: 14 }, (_, i) => addDays(day, i));
  const today = todayKey();
  return (
    <div className="agenda">
      {days.map((d) => {
        const list = eventsOn(events, d);
        if (!list.length) return null;
        return (
          <section key={dayKey(d)} className="agenda__day">
            <div className={`agenda__date${dayKey(d) === today ? " agenda__date--today" : ""}`}><b>{d.getDate()}</b><span>{DOW[(d.getDay() + 6) % 7]}, {MONTHS[d.getMonth()]!.slice(0, 3)}</span></div>
            <div className="agenda__list">
              {list.map((e) => (
                <button key={calKey(e)} type="button" className="agenda__ev" onClick={() => selectEvent(calKey(e))}>
                  <span className="cal__swatch" style={{ background: colorOf[e.calendarId ?? "primary"] ?? "#7986cb" }} />
                  <span className="agenda__time">{fmtRange(e)}</span>
                  <span className="agenda__title">{e.summary ?? "(No title)"}</span>
                  {meetLinkOf(e) && <Icon n="video" size={14} />}
                </button>
              ))}
            </div>
          </section>
        );
      })}
      {days.every((d) => eventsOn(events, d).length === 0) && <div className="feed__empty"><Icon n="calendar" size={28} /><b>Nothing in the next two weeks</b>Double-click a slot in the week view, or press New event.</div>}
    </div>
  );
}

/** One event's details: when, where, who and how they answered, Join, respond, edit, delete. */
function EventPopover({ eventKey }: { eventKey: string }) {
  const e = useStore((s) => s.calendar.events[eventKey]);
  const me = useStore((s) => s.auth.user);
  const { mounted, exiting } = useExit(!!e, 120);
  if (!mounted || !e) return null;
  const mine = myResponse(e);
  const link = meetLinkOf(e);
  const own = !!e.organizer?.self || (e.attendees ?? []).some((a) => a.self && a.organizer) || (!e.attendees?.length && !e.organizer);
  const counts = { accepted: 0, declined: 0, tentative: 0, needsAction: 0 } as Record<string, number>;
  for (const a of e.attendees ?? []) counts[a.responseStatus ?? "needsAction"] = (counts[a.responseStatus ?? "needsAction"] ?? 0) + 1;
  return (
    <Menu className="menu--event" label={e.summary ?? "Event"} exiting={exiting} onClose={() => selectEvent(undefined)}>
      <div className="evp__head">
        <div className="evp__title">{e.summary ?? "(No title)"}</div>
        <div className="evp__when">{fmtDayLong(startOf(e))} · {fmtRange(e)}{e.recurrence?.length ? ` · ${describeRrule(e.recurrence)}` : e.recurringEventId ? " · Repeats" : ""}</div>
        {e.location && <div className="evp__row"><Icon n="globe" size={14} />{e.location}</div>}
        {e.description && <div className="evp__desc">{e.description}</div>}
      </div>
      {link && <button type="button" className="btn btn--primary evp__join" onClick={() => void openExternal(link)}><Icon n="video" />Join with Google Meet</button>}
      {(e.attendees?.length ?? 0) > 0 && (
        <div className="evp__people">
          <div className="evp__count">{e.attendees!.length} guests · {counts.accepted} yes{counts.tentative ? `, ${counts.tentative} maybe` : ""}{counts.declined ? `, ${counts.declined} no` : ""}{counts.needsAction ? `, ${counts.needsAction} awaiting` : ""}</div>
          {e.attendees!.slice(0, 8).map((a) => (
            <div key={a.email} className="evp__person">
              <Avatar user={{ name: a.email, displayName: a.displayName ?? a.email }} size={22} />
              <span>{a.displayName ?? a.email}{a.organizer ? " · organizer" : ""}{a.self ? " (you)" : ""}</span>
              <span className={`evp__rsvp evp__rsvp--${a.responseStatus ?? "needsAction"}`}>{a.responseStatus === "accepted" ? "Yes" : a.responseStatus === "declined" ? "No" : a.responseStatus === "tentative" ? "Maybe" : "?"}</span>
            </div>
          ))}
        </div>
      )}
      {(e.attendees ?? []).some((a) => a.self && !a.organizer) && (
        <div className="evp__respond">
          <span>Going?</span>
          {(["accepted", "tentative", "declined"] as const).map((r) => (
            <button key={r} type="button" className={`btn${mine === r ? " btn--primary" : ""}`} onClick={() => void respondEvent(e, r)}>{r === "accepted" ? "Yes" : r === "tentative" ? "Maybe" : "No"}</button>
          ))}
        </div>
      )}
      <div className="menu__sep" />
      <button type="button" role="menuitem" className="menu__item" onClick={() => editEvent(e)}><Icon n="edit" />{own ? "Edit" : "View details"}</button>
      {e.htmlLink && <button type="button" role="menuitem" className="menu__item" onClick={() => { selectEvent(undefined); void openExternal(e.htmlLink!); }}><Icon n="external" />Open in Google Calendar</button>}
      {own && <button type="button" role="menuitem" className="menu__item menu__item--danger" onClick={() => { if (window.confirm(`Delete "${e.summary ?? "this event"}"? Guests are told.`)) void deleteEvent(e); }}><Icon n="trash" />Delete</button>}
      {me && !own && e.organizer?.email && <div className="menu__note">Organised by {e.organizer.displayName ?? e.organizer.email}.</div>}
    </Menu>
  );
}

const toLocalInput = (iso: string) => { const d = new Date(iso); return `${dayKey(d)}T${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`; };
const roundUp = (d: Date) => { const x = new Date(d); x.setSeconds(0, 0); x.setMinutes(Math.ceil(x.getMinutes() / 30) * 30); return x; };

/** Create or edit: title, when (or all day), guests, Meet, repeat, where, details. */
function EventDialog({ exiting }: { exiting?: boolean }) {
  const editing = useStore((s) => s.calendar.editing);
  const calendars = useStore((s) => s.calendar.calendars);
  const existing = editing && "id" in editing && editing.id ? (editing as CalEvent) : undefined;
  const members = useStore((s) => s.members);
  const startInit = editing?.start?.dateTime ? new Date(editing.start.dateTime) : editing?.start?.date ? fromKey(editing.start.date) : roundUp(new Date());
  const endInit = editing?.end?.dateTime ? new Date(editing.end.dateTime) : editing?.end?.date ? fromKey(editing.end.date) : new Date(startInit.getTime() + 30 * 60000);
  const [title, setTitle] = useState(editing?.summary ?? "");
  const [allDay, setAllDay] = useState(!!editing?.start?.date);
  const [start, setStart] = useState(toLocalInput(startInit.toISOString()));
  const [end, setEnd] = useState(toLocalInput(endInit.toISOString()));
  const [where, setWhere] = useState(editing?.location ?? "");
  const [desc, setDesc] = useState(editing?.description ?? "");
  const [meet, setMeet] = useState(!!existing && !!meetLinkOf(existing));
  const [repeat, setRepeat] = useState<"none" | "daily" | "weekdays" | "weekly" | "monthly" | "yearly">(existing?.recurrence?.length ? (describeRrule(existing.recurrence) === "Every day" ? "daily" : describeRrule(existing.recurrence) === "Every weekday" ? "weekdays" : describeRrule(existing.recurrence) === "Every week" ? "weekly" : describeRrule(existing.recurrence) === "Every month" ? "monthly" : "yearly") : "none");
  const [calendarId, setCalendarId] = useState(existing?.calendarId ?? calendars.find((c) => c.primary)?.id ?? calendars[0]?.id ?? "primary");
  const known = useMemo(() => { const out: Record<string, User> = {}; for (const list of Object.values(members)) for (const m of list) if (m.member?.email) out[m.member.email] = m.member; return out; }, [members]);
  const [guests, setGuests] = useState<User[]>((existing?.attendees ?? []).filter((a) => !a.self).map((a) => known[a.email] ?? { name: a.email, displayName: a.displayName, email: a.email }));
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState("");
  const readOnly = !!existing && !(existing.organizer?.self || (existing.attendees ?? []).some((a) => a.self && a.organizer) || (!existing.attendees?.length && !existing.organizer));
  const submit = (e: FormEvent) => {
    e.preventDefault();
    if (!title.trim() || busy || readOnly) return;
    const s = new Date(start);
    const en = new Date(end);
    if (!allDay && en <= s) { setErr("The end is before the start."); return; }
    const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
    const input: EventInput = {
      summary: title.trim(), description: desc.trim() || undefined, location: where.trim() || undefined,
      start: allDay ? { date: dayKey(s) } : { dateTime: s.toISOString(), timeZone: tz },
      end: allDay ? { date: dayKey(addDays(en < s ? s : en, allDay ? 1 : 0)) } : { dateTime: en.toISOString(), timeZone: tz },
      attendees: guests.map((g) => g.email ?? g.name).filter((x) => x.includes("@")),
      addMeet: meet, recurrence: rrule(repeat, s),
    };
    setBusy(true);
    setErr("");
    saveEvent(calendarId, input, existing).catch((x: unknown) => { setErr((x as Error)?.message ?? String(x)); setBusy(false); });
  };
  return (
    <Modal title={readOnly ? "Event" : existing ? "Edit event" : "New event"} onClose={closeEventEditor} exiting={exiting}>
      <form onSubmit={submit}>
        <div className="dialog__body evd">
          <input className="evd__title" autoFocus value={title} onChange={(e) => setTitle(e.target.value)} placeholder="Add a title" aria-label="Title" readOnly={readOnly} required />
          <div className="evd__row">
            <Icon n="clock" size={16} />
            <input type={allDay ? "date" : "datetime-local"} value={allDay ? start.slice(0, 10) : start} onChange={(e) => setStart(allDay ? `${e.target.value}T09:00` : e.target.value)} aria-label="Start" readOnly={readOnly} />
            <span>to</span>
            <input type={allDay ? "date" : "datetime-local"} value={allDay ? end.slice(0, 10) : end} onChange={(e) => setEnd(allDay ? `${e.target.value}T10:00` : e.target.value)} aria-label="End" readOnly={readOnly} />
            <label className="evd__check"><input type="checkbox" checked={allDay} onChange={(e) => setAllDay(e.target.checked)} disabled={readOnly} />All day</label>
          </div>
          <div className="evd__row">
            <Icon n="calendar" size={16} />
            <select value={repeat} onChange={(e) => setRepeat(e.target.value as typeof repeat)} aria-label="Repeat" disabled={readOnly}>
              <option value="none">Does not repeat</option><option value="daily">Every day</option><option value="weekdays">Every weekday</option><option value="weekly">Every week</option><option value="monthly">Every month</option><option value="yearly">Every year</option>
            </select>
            <select value={calendarId} onChange={(e) => setCalendarId(e.target.value)} aria-label="Calendar" disabled={!!existing}>
              {calendars.map((c) => <option key={c.id} value={c.id}>{c.summary}</option>)}
            </select>
          </div>
          <div className="evd__row evd__row--top">
            <Icon n="users" size={16} />
            <div className="evd__guests">{readOnly ? <div className="evd__static">{guests.map(userLabel).join(", ") || "No guests"}</div> : <PeoplePicker value={guests} onChange={setGuests} />}</div>
          </div>
          <div className="evd__row">
            <Icon n="video" size={16} />
            <label className="evd__check"><input type="checkbox" checked={meet} onChange={(e) => setMeet(e.target.checked)} disabled={readOnly || (!!existing && !!meetLinkOf(existing))} />{existing && meetLinkOf(existing) ? "Google Meet attached" : "Add Google Meet video conferencing"}</label>
          </div>
          <div className="evd__row"><Icon n="globe" size={16} /><input value={where} onChange={(e) => setWhere(e.target.value)} placeholder="Add location" aria-label="Location" readOnly={readOnly} /></div>
          <div className="evd__row evd__row--top"><Icon n="text" size={16} /><textarea value={desc} onChange={(e) => setDesc(e.target.value)} placeholder="Add description" aria-label="Description" rows={3} readOnly={readOnly} /></div>
          {err && <div className="dialog__note dialog__note--err" role="alert">{err}</div>}
        </div>
        <div className="dialog__actions">
          {existing && !readOnly && <button type="button" className="btn btn--danger" onClick={() => { if (window.confirm("Delete this event? Guests are told.")) void deleteEvent(existing); }}>Delete</button>}
          <span className="header__spacer" />
          <button type="button" className="btn" onClick={closeEventEditor} disabled={busy}>{readOnly ? "Close" : "Cancel"}</button>
          {!readOnly && <button type="submit" className="btn btn--primary" disabled={busy || !title.trim()}>{busy ? "Saving…" : existing ? "Save" : guests.length ? "Send invites" : "Save"}</button>}
        </div>
      </form>
    </Modal>
  );
}

export { endOf };
