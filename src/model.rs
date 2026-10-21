//! Calendar data as the UI sees it, independent of where it came from (Google or demo).

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, Zoned};

#[derive(Clone, Debug, PartialEq)]
pub struct Calendar {
    /// Unique across accounts: [`Calendar::key`] of the account and the Google calendar id.
    /// Two accounts can subscribe to the same calendar, so the Google id alone is ambiguous.
    pub id: String,
    /// The Google account (email) this calendar was listed under.
    pub account: String,
    /// Google's calendar id, used in API paths.
    pub remote_id: String,
    pub name: String,
    /// 0xRRGGBB.
    pub color: u32,
    pub primary: bool,
    /// Popup reminder offsets (minutes before start) for events with `useDefault`.
    pub default_reminders: Vec<i64>,
    /// Google's own "shown in the list" flag; the initial visibility when the user never toggled it.
    pub selected_upstream: bool,
}

impl Calendar {
    /// Emails never contain '/', so the key splits unambiguously on its first '/'.
    pub fn key(account: &str, remote_id: &str) -> String {
        format!("{account}/{remote_id}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum When {
    /// `end` is exclusive, as in iCalendar and the Google API.
    AllDay {
        start: Date,
        end: Date,
    },
    Timed {
        start: Timestamp,
        end: Timestamp,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Response {
    Accepted,
    Tentative,
    Declined,
    NeedsAction,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Attendee {
    pub name: String,
    pub email: String,
    pub response: Response,
    pub organizer: bool,
    pub is_self: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub id: String,
    pub calendar_id: String,
    pub title: String,
    pub when: When,
    /// Event colour override (0xRRGGBB); `None` uses the calendar colour.
    pub color: Option<u32>,
    pub location: Option<String>,
    /// Plain text (HTML from Google is flattened on import).
    pub description: Option<String>,
    pub html_link: Option<String>,
    pub meet_link: Option<String>,
    pub organizer: Option<String>,
    pub attendees: Vec<Attendee>,
    /// Popup reminder offsets in minutes, already resolved against the calendar defaults.
    pub reminders: Vec<i64>,
    pub tentative: bool,
}

impl Event {
    /// The current user's RSVP, when they are an attendee.
    pub fn self_response(&self) -> Option<Response> {
        self.attendees
            .iter()
            .find(|a| a.is_self)
            .map(|a| a.response)
    }

    pub fn declined(&self) -> bool {
        self.self_response() == Some(Response::Declined)
    }

    pub fn is_all_day(&self) -> bool {
        matches!(self.when, When::AllDay { .. })
    }

    /// Start and (exclusive) end as instants. All-day events start and end at local midnight.
    pub fn instants(&self, tz: &TimeZone) -> (Timestamp, Timestamp) {
        match self.when {
            When::Timed { start, end } => (start, end.max(start)),
            When::AllDay { start, end } => (
                day_start(start, tz).timestamp(),
                day_start(end.max(start.tomorrow().unwrap_or(start)), tz).timestamp(),
            ),
        }
    }

    /// Whether any part of the event falls on `date` in `tz`. Zero-length events count on the
    /// day they start; an event ending exactly at midnight does not spill into the next day.
    pub fn occurs_on(&self, date: Date, tz: &TimeZone) -> bool {
        let (start, end) = self.instants(tz);
        let day_from = day_start(date, tz).timestamp();
        let day_to = day_start(date.tomorrow().unwrap_or(date), tz).timestamp();
        if start == end {
            start >= day_from && start < day_to
        } else {
            start < day_to && end > day_from
        }
    }

    pub fn start_zoned(&self, tz: &TimeZone) -> Zoned {
        self.instants(tz).0.to_zoned(tz.clone())
    }

    pub fn end_zoned(&self, tz: &TimeZone) -> Zoned {
        self.instants(tz).1.to_zoned(tz.clone())
    }
}

/// Local midnight (or the first instant of the day, across DST gaps).
pub fn day_start(date: Date, tz: &TimeZone) -> Zoned {
    date.to_zoned(tz.clone())
        .unwrap_or_else(|_| Timestamp::UNIX_EPOCH.to_zoned(tz.clone()))
}

pub fn first_of_month(date: Date) -> Date {
    date.first_of_month()
}

pub fn add_months(date: Date, months: i32) -> Date {
    date.checked_add(months.months()).unwrap_or(date)
}

pub fn add_days(date: Date, days: i32) -> Date {
    date.checked_add(days.days()).unwrap_or(date)
}

/// Monday of the ISO week containing `date`.
pub fn week_start(date: Date) -> Date {
    let offset = date.weekday().to_monday_zero_offset();
    add_days(date, -(offset as i32))
}

/// The 42 days shown in a month grid (6 weeks starting on the Monday on or before the 1st).
pub fn month_grid(month: Date) -> Vec<Date> {
    let first = week_start(first_of_month(month));
    (0..42).map(|i| add_days(first, i)).collect()
}

/// Months (as their first day) touched by the half-open date range `[from, to)`.
pub fn months_between(from: Date, to: Date) -> Vec<Date> {
    let mut out = vec![];
    let mut m = first_of_month(from);
    while m < to {
        out.push(m);
        m = add_months(m, 1);
    }
    if out.is_empty() {
        out.push(first_of_month(from));
    }
    out
}

/// The Google event colour palette (`colorId` 1–11), fixed by the API.
pub fn google_event_color(color_id: &str) -> Option<u32> {
    Some(match color_id {
        "1" => 0x7986cb,
        "2" => 0x33b679,
        "3" => 0x8e24aa,
        "4" => 0xe67c73,
        "5" => 0xf6bf26,
        "6" => 0xf4511e,
        "7" => 0x039be5,
        "8" => 0x616161,
        "9" => 0x3f51b5,
        "10" => 0x0b8043,
        "11" => 0xd50000,
        _ => return None,
    })
}

pub fn parse_hex_color(s: &str) -> Option<u32> {
    let hex = s.strip_prefix('#')?;
    (hex.len() == 6).then(|| u32::from_str_radix(hex, 16).ok())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn timed(start: &str, end: &str) -> Event {
        Event {
            id: "e".into(),
            calendar_id: "c".into(),
            title: "t".into(),
            when: When::Timed {
                start: start.parse().unwrap(),
                end: end.parse().unwrap(),
            },
            color: None,
            location: None,
            description: None,
            html_link: None,
            meet_link: None,
            organizer: None,
            attendees: vec![],
            reminders: vec![],
            tentative: false,
        }
    }

    #[test]
    fn event_ending_at_midnight_stays_on_its_day() {
        let tz = TimeZone::UTC;
        let e = timed("2026-10-21T22:00:00Z", "2026-10-22T00:00:00Z");
        assert!(e.occurs_on(date(2026, 10, 21), &tz));
        assert!(!e.occurs_on(date(2026, 10, 22), &tz));
    }

    #[test]
    fn overnight_event_spans_both_days() {
        let tz = TimeZone::UTC;
        let e = timed("2026-10-21T22:00:00Z", "2026-10-22T01:00:00Z");
        assert!(e.occurs_on(date(2026, 10, 21), &tz));
        assert!(e.occurs_on(date(2026, 10, 22), &tz));
    }

    #[test]
    fn zero_length_event_counts_on_its_start_day() {
        let tz = TimeZone::UTC;
        let e = timed("2026-10-22T00:00:00Z", "2026-10-22T00:00:00Z");
        assert!(!e.occurs_on(date(2026, 10, 21), &tz));
        assert!(e.occurs_on(date(2026, 10, 22), &tz));
    }

    #[test]
    fn all_day_end_is_exclusive() {
        // A fixed zone: the Nix build sandbox has no tz database.
        let tz = TimeZone::fixed(jiff::tz::offset(1));
        let mut e = timed("2026-10-21T00:00:00Z", "2026-10-21T00:00:00Z");
        e.when = When::AllDay {
            start: date(2026, 10, 21),
            end: date(2026, 10, 23),
        };
        assert!(!e.occurs_on(date(2026, 10, 20), &tz));
        assert!(e.occurs_on(date(2026, 10, 21), &tz));
        assert!(e.occurs_on(date(2026, 10, 22), &tz));
        assert!(!e.occurs_on(date(2026, 10, 23), &tz));
    }

    #[test]
    fn month_grid_starts_on_monday_and_covers_month() {
        let grid = month_grid(date(2026, 10, 21));
        assert_eq!(grid[0], date(2026, 9, 28));
        assert_eq!(grid.len(), 42);
        assert!(grid.contains(&date(2026, 10, 31)));
    }

    #[test]
    fn months_between_is_half_open() {
        assert_eq!(
            months_between(date(2026, 9, 28), date(2026, 11, 9)),
            vec![date(2026, 9, 1), date(2026, 10, 1), date(2026, 11, 1)]
        );
        assert_eq!(
            months_between(date(2026, 10, 5), date(2026, 11, 1)),
            vec![date(2026, 10, 1)]
        );
    }

    #[test]
    fn hex_colors() {
        assert_eq!(parse_hex_color("#9fe1e7"), Some(0x9fe1e7));
        assert_eq!(parse_hex_color("9fe1e7"), None);
        assert_eq!(parse_hex_color("#fff"), None);
    }
}
