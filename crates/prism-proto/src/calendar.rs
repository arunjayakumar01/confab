//! The Google Calendar API v3 subset Prism uses: the calendar list, events in a
//! window or by sync token, and what the client sends to create or update one.
//! JSON camelCase on the wire, Google's field names; `calendarId` on an event
//! is Prism's own, filled by the desktop with the calendar the event came from.

use serde::{Deserialize, Serialize};

/// One row of `GET users/me/calendarList`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEntry {
    /// `primary` for the signed-in user's own calendar, else an email-shaped id.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_color: Option<String>,
    /// `owner` | `writer` | `reader` | `freeBusyReader`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_role: Option<String>,
}

/// An event boundary: `dateTime` (RFC 3339 with offset) for timed events, `date`
/// (`YYYY-MM-DD`) for all-day ones; `timeZone` is optional either way.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventTime {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
}

impl EventTime {
    pub fn is_all_day(&self) -> bool {
        self.date.is_some() && self.date_time.is_none()
    }
}

/// One attendee; `self` is relative to the signed-in user.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Attendee {
    #[serde(default)]
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// `needsAction` | `accepted` | `declined` | `tentative`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_status: Option<String>,
    #[serde(default, rename = "self")]
    pub is_self: bool,
    #[serde(default)]
    pub organizer: bool,
    #[serde(default)]
    pub optional: bool,
}

impl Attendee {
    pub const NEEDS_ACTION: &'static str = "needsAction";
    pub const ACCEPTED: &'static str = "accepted";
    pub const DECLINED: &'static str = "declined";
    pub const TENTATIVE: &'static str = "tentative";

    /// The three answers a person can give to an invitation.
    pub fn is_response(s: &str) -> bool {
        matches!(s, Self::ACCEPTED | Self::DECLINED | Self::TENTATIVE)
    }
}

/// One way into a conference, e.g. `{entryPointType: "video", uri: "https://meet.google.com/..."}`.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntryPoint {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_point_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConferenceData {
    #[serde(default)]
    pub entry_points: Vec<EntryPoint>,
}

/// `GET|POST|PATCH calendars/{id}/events[/{eventId}]`, the fields Prism reads.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CalEvent {
    #[serde(default)]
    pub id: String,
    /// `confirmed` | `tentative` | `cancelled` (deleted events in a sync answer)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub html_link: Option<String>,
    /// The Meet link when the event has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hangout_link: Option<String>,
    #[serde(default)]
    pub start: EventTime,
    #[serde(default)]
    pub end: EventTime,
    #[serde(default)]
    pub attendees: Vec<Attendee>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub organizer: Option<Attendee>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conference_data: Option<ConferenceData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
    /// The series this instance expands (`singleEvents=true`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurring_event_id: Option<String>,
    /// RRULE, EXRULE, RDATE and EXDATE lines of a series, e.g. `RRULE:FREQ=WEEKLY;BYDAY=MO,WE`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recurrence: Vec<String>,
    /// `opaque` (busy, the default) | `transparent` (free)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transparency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_id: Option<String>,
    /// Prism's own: the calendar the event was listed from, filled by the desktop.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calendar_id: Option<String>,
}

impl CalEvent {
    pub const CONFIRMED: &'static str = "confirmed";
    pub const CANCELLED: &'static str = "cancelled";

    pub fn is_cancelled(&self) -> bool {
        self.status.as_deref() == Some(Self::CANCELLED)
    }
}

/// `GET calendars/{id}/events`, pages merged by the desktop: `nextSyncToken` is
/// the token for the next incremental fetch, `nextPageToken` only survives on the wire
/// between the desktop and Google.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventsPage {
    #[serde(default)]
    pub items: Vec<CalEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_sync_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_page_token: Option<String>,
}

/// What the web view sends to create or update an event. `attendees` are emails;
/// `start` and `end` carry `dateTime` or, for an all-day event, `date`; `recurrence`
/// is the RRULE lines of a series; `addMeet` asks for a Meet conference.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EventInput {
    #[serde(default)]
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    #[serde(default)]
    pub start: EventTime,
    #[serde(default)]
    pub end: EventTime,
    #[serde(default)]
    pub attendees: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recurrence: Vec<String>,
    #[serde(default)]
    pub add_meet: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_json_uses_google_names_and_drops_none() {
        let raw = r#"{"kind":"calendar#event","id":"ev-1","status":"confirmed","summary":"Standup",
            "htmlLink":"https://calendar.google.com/event?eid=ev-1",
            "hangoutLink":"https://meet.google.com/abc-defg-hij",
            "start":{"dateTime":"2026-11-07T10:00:00+05:30"},"end":{"dateTime":"2026-11-07T10:15:00+05:30"},
            "attendees":[{"email":"arun@prism.test","self":true,"organizer":true,"responseStatus":"accepted"},
                         {"email":"rahul@prism.test","responseStatus":"needsAction","optional":true}],
            "organizer":{"email":"arun@prism.test","displayName":"Arun Jayakumar","self":true},
            "conferenceData":{"entryPoints":[{"entryPointType":"video","uri":"https://meet.google.com/abc-defg-hij"}],
                              "conferenceSolution":{"name":"Google Meet"}},
            "recurrence":["RRULE:FREQ=WEEKLY;BYDAY=MO,WE"],"transparency":"transparent",
            "created":"2026-11-01T00:00:00.000Z","updated":"2026-11-01T00:00:00.000Z"}"#;
        let ev: CalEvent = serde_json::from_str(raw).unwrap();
        assert_eq!(ev.id, "ev-1");
        assert_eq!(ev.summary.as_deref(), Some("Standup"));
        assert!(!ev.is_cancelled());
        assert_eq!(ev.attendees.len(), 2);
        assert!(ev.attendees[0].is_self && ev.attendees[0].organizer);
        assert!(ev.attendees[1].optional && !ev.attendees[1].is_self);
        assert_eq!(
            ev.organizer.as_ref().unwrap().display_name.as_deref(),
            Some("Arun Jayakumar")
        );
        assert_eq!(
            ev.conference_data.as_ref().unwrap().entry_points[0]
                .uri
                .as_deref(),
            Some("https://meet.google.com/abc-defg-hij")
        );
        assert_eq!(
            ev.recurrence,
            vec!["RRULE:FREQ=WEEKLY;BYDAY=MO,WE".to_string()]
        );
        assert_eq!(ev.transparency.as_deref(), Some("transparent"));
        assert!(ev.calendar_id.is_none());
        assert!(!ev.start.is_all_day());

        let v = serde_json::to_value(&ev).unwrap();
        assert_eq!(
            v["start"],
            serde_json::json!({ "dateTime": "2026-11-07T10:00:00+05:30" })
        );
        assert_eq!(v["attendees"][1]["self"], false);
        assert!(v.get("description").is_none());
        assert!(v.get("calendarId").is_none());
        assert_eq!(v["recurrence"][0], "RRULE:FREQ=WEEKLY;BYDAY=MO,WE");
        let back: CalEvent = serde_json::from_value(v).unwrap();
        assert_eq!(back, ev);

        let cancelled: CalEvent =
            serde_json::from_str(r#"{"id":"ev-2","status":"cancelled"}"#).unwrap();
        assert!(cancelled.is_cancelled());
        assert!(cancelled.attendees.is_empty());
        assert!(cancelled.recurrence.is_empty());
        let plain = serde_json::to_value(&cancelled).unwrap();
        assert!(plain.get("recurrence").is_none());
        assert_eq!(plain["attendees"], serde_json::json!([]));

        let page: EventsPage =
            serde_json::from_str(r#"{"kind":"calendar#events","items":[],"nextSyncToken":"7"}"#)
                .unwrap();
        assert!(page.items.is_empty());
        assert_eq!(page.next_sync_token.as_deref(), Some("7"));
        assert_eq!(
            serde_json::to_value(&page).unwrap(),
            serde_json::json!({ "items": [], "nextSyncToken": "7" })
        );
    }

    #[test]
    fn calendar_entry_and_input_round_trip() {
        let entry: CalendarEntry = serde_json::from_str(
            r##"{"id":"primary","summary":"Arun","primary":true,"backgroundColor":"#7986cb","accessRole":"owner"}"##,
        )
        .unwrap();
        assert!(entry.primary);
        assert_eq!(entry.access_role.as_deref(), Some("owner"));
        let team: CalendarEntry =
            serde_json::from_str(r#"{"id":"team@example.test","summary":"Team"}"#).unwrap();
        assert!(!team.primary);
        assert_eq!(
            serde_json::to_value(&team).unwrap(),
            serde_json::json!({ "id": "team@example.test", "summary": "Team", "primary": false })
        );

        let input: EventInput = serde_json::from_str(
            r#"{"summary":"Sync","start":{"dateTime":"2026-11-08T09:00:00+05:30"},
                "end":{"dateTime":"2026-11-08T09:30:00+05:30"},"attendees":["priya@prism.test"],"addMeet":true}"#,
        )
        .unwrap();
        assert!(input.add_meet);
        assert_eq!(input.attendees, vec!["priya@prism.test".to_string()]);
        assert!(input.recurrence.is_empty());
        assert_eq!(
            serde_json::to_value(&input).unwrap(),
            serde_json::json!({
                "summary": "Sync",
                "start": { "dateTime": "2026-11-08T09:00:00+05:30" },
                "end": { "dateTime": "2026-11-08T09:30:00+05:30" },
                "attendees": ["priya@prism.test"],
                "addMeet": true
            })
        );

        let all_day: EventInput = serde_json::from_str(
            r#"{"summary":"Offsite","start":{"date":"2026-11-20"},"end":{"date":"2026-11-21"},
                "recurrence":["RRULE:FREQ=WEEKLY;BYDAY=MO,WE"]}"#,
        )
        .unwrap();
        assert!(all_day.start.is_all_day() && all_day.end.is_all_day());
        assert!(!all_day.add_meet);
        assert_eq!(all_day.recurrence.len(), 1);
        assert!(Attendee::is_response("tentative"));
        assert!(!Attendee::is_response("needsAction"));
    }
}
