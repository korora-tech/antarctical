//! Popup reminders from Google's reminder settings, shown as desktop notifications.

use std::collections::HashSet;

use jiff::tz::TimeZone;
use jiff::{SignedDuration, Timestamp};

use crate::model::Event;

/// A reminder that became due while Antarctical was not running is still shown if it is at most
/// this old, and never after its event has started plus this much.
const STALE_AFTER: SignedDuration = SignedDuration::from_mins(10);

#[derive(Debug, PartialEq, Eq)]
pub struct Due {
    pub title: String,
    pub body: String,
    pub meet_link: Option<String>,
}

/// Reminders due at `now` that have not fired yet; marks them fired.
pub fn due<'a>(
    events: impl IntoIterator<Item = &'a Event>,
    now: Timestamp,
    tz: &TimeZone,
    fired: &mut HashSet<String>,
) -> Vec<Due> {
    let mut out = vec![];
    for e in events {
        if e.declined() {
            continue;
        }
        let span = e.instants(tz);
        for &minutes in &e.reminders {
            let fire_at = span.start - SignedDuration::from_mins(minutes);
            if fire_at > now || now.duration_since(fire_at) > STALE_AFTER {
                continue;
            }
            if now.duration_since(span.start) > STALE_AFTER {
                continue;
            }
            let key = format!("{}/{}/{}/{}", e.calendar_id, e.id, span.start, minutes);
            if !fired.insert(key) {
                continue;
            }
            out.push(Due {
                title: e.title.clone(),
                body: body(e, now, span.start, span.end, tz),
                meet_link: e.meet_link.clone(),
            });
        }
    }
    out
}

fn body(e: &Event, now: Timestamp, start: Timestamp, end: Timestamp, tz: &TimeZone) -> String {
    let mins = start.duration_since(now).as_secs().div_euclid(60);
    let when = if e.is_all_day() {
        "Today".to_string()
    } else if mins > 0 {
        let fmt = |t: Timestamp| t.to_zoned(tz.clone()).strftime("%H:%M").to_string();
        let rel = if mins >= 60 {
            format!("In {}h {:02}m", mins / 60, mins % 60)
        } else {
            format!("In {mins} min")
        };
        format!("{rel} · {}–{}", fmt(start), fmt(end))
    } else {
        "Starting now".to_string()
    };
    match &e.location {
        Some(loc) => format!("{when} · {loc}"),
        None => when,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Attendee, Response, When};

    fn event(start: &str, reminders: Vec<i64>) -> Event {
        let start: Timestamp = start.parse().unwrap();
        Event {
            id: "e".into(),
            calendar_id: "c".into(),
            title: "Standup".into(),
            when: When::Timed {
                start,
                end: start + SignedDuration::from_mins(15),
            },
            color: None,
            location: Some("Room 4".into()),
            description: None,
            html_link: None,
            meet_link: None,
            organizer: None,
            attendees: vec![],
            reminders,
            tentative: false,
        }
    }

    #[test]
    fn fires_once_when_due() {
        let tz = TimeZone::UTC;
        let e = event("2026-10-21T09:30:00Z", vec![10]);
        let mut fired = HashSet::new();
        let early: Timestamp = "2026-10-21T09:19:00Z".parse().unwrap();
        assert!(due([&e], early, &tz, &mut fired).is_empty());
        let now: Timestamp = "2026-10-21T09:20:30Z".parse().unwrap();
        let d = due([&e], now, &tz, &mut fired);
        assert_eq!(d.len(), 1);
        assert_eq!(d[0].body, "In 9 min · 09:30–09:45 · Room 4");
        assert!(due([&e], now, &tz, &mut fired).is_empty());
    }

    #[test]
    fn stale_and_declined_are_skipped() {
        let tz = TimeZone::UTC;
        let mut fired = HashSet::new();
        let e = event("2026-10-21T09:30:00Z", vec![60]);
        let late: Timestamp = "2026-10-21T08:45:00Z".parse().unwrap();
        assert!(due([&e], late, &tz, &mut fired).is_empty());
        let mut d = event("2026-10-21T09:30:00Z", vec![10]);
        d.attendees = vec![Attendee {
            name: "me".into(),
            email: "me".into(),
            display_name: None,
            comment: None,
            additional_guests: None,
            optional: None,
            resource: None,
            response: Response::Declined,
            organizer: false,
            is_self: true,
        }];
        let now: Timestamp = "2026-10-21T09:21:00Z".parse().unwrap();
        assert!(due([&d], now, &tz, &mut fired).is_empty());
    }

    #[test]
    fn zero_minute_reminder_says_starting_now() {
        let tz = TimeZone::UTC;
        let mut fired = HashSet::new();
        let e = event("2026-10-21T09:30:00Z", vec![0]);
        let now: Timestamp = "2026-10-21T09:30:20Z".parse().unwrap();
        assert_eq!(
            due([&e], now, &tz, &mut fired)[0].body,
            "Starting now · Room 4"
        );
    }
}
