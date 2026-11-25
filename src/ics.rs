//! A small reader for iCalendar (`.ics`) files: just the events, as far as importing them into
//! Google Calendar needs. Guests, alarms and per-occurrence overrides are deliberately ignored.

use jiff::SignedDuration;
use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;

use crate::model::When;

/// The most events one import takes.
pub const MAX_EVENTS: usize = 200;
/// The largest file read.
pub const MAX_BYTES: u64 = 1 << 20;

#[derive(Clone, Debug, PartialEq)]
pub struct Imported {
    pub title: String,
    pub when: When,
    pub location: Option<String>,
    pub description: Option<String>,
    /// An `RRULE` body (`FREQ=WEEKLY;BYDAY=MO`). Only kept when the zone it runs in is known.
    pub rrule: Option<String>,
    /// The IANA zone the times are in (timed events only).
    pub time_zone: Option<String>,
}

#[derive(Debug, Default, PartialEq)]
pub struct Parsed {
    pub events: Vec<Imported>,
    /// Entries left out: cancelled ones, single-occurrence overrides, ones without a usable
    /// start, and anything past [`MAX_EVENTS`].
    pub skipped: usize,
}

impl Imported {
    /// The event as it would be created: no guests (importing never sends invitations), no Meet
    /// link, default reminders.
    pub fn draft(&self) -> crate::google::api::EventDraft {
        crate::google::api::EventDraft {
            title: self.title.clone(),
            when: self.when,
            time_zone: self.time_zone.clone(),
            location: self.location.clone(),
            description: self.description.clone(),
            guests: vec![],
            meet: false,
            rrule: self.rrule.clone(),
            edit_guests: None,
            edit_reminders: None,
            series: None,
        }
    }
}

/// Joins folded lines: a line starting with a space or tab continues the one before it.
fn unfold(text: &str) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for raw in text.lines() {
        match (raw.strip_prefix([' ', '\t']), lines.last_mut()) {
            (Some(rest), Some(last)) => last.push_str(rest),
            _ => lines.push(raw.to_string()),
        }
    }
    lines
}

/// One content line: name, parameters, value.
type Prop = (String, Vec<(String, String)>, String);

/// `NAME;PARAM=x;PARAM=y:value` into (NAME, params, value).
fn split_property(line: &str) -> Option<Prop> {
    let mut in_quotes = false;
    let colon = line.char_indices().find_map(|(i, c)| {
        match c {
            '"' => in_quotes = !in_quotes,
            ':' if !in_quotes => return Some(i),
            _ => {}
        }
        None
    })?;
    let (head, value) = (&line[..colon], &line[colon + 1..]);
    let mut parts = head.split(';');
    let name = parts.next()?.trim().to_uppercase();
    let params = parts
        .filter_map(|p| {
            let (k, v) = p.split_once('=')?;
            Some((k.to_uppercase(), v.trim_matches('"').to_string()))
        })
        .collect();
    Some((name, params, value.to_string()))
}

fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// A date or date-time value: whole date, UTC instant, or a wall-clock time in `zone`.
fn parse_time(
    value: &str,
    params: &[(String, String)],
    local: &TimeZone,
    lookup: &dyn Fn(&str) -> Option<TimeZone>,
) -> Option<(When, Option<String>)> {
    let param = |name: &str| {
        params
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    };
    let value = value.trim();
    if param("VALUE") == Some("DATE") || (value.len() == 8 && !value.contains('T')) {
        let start = Date::strptime("%Y%m%d", value).ok()?;
        return Some((
            When::AllDay {
                start,
                end: start.tomorrow().ok()?,
            },
            None,
        ));
    }
    if let Some(utc) = value.strip_suffix('Z') {
        let at = DateTime::strptime("%Y%m%dT%H%M%S", utc).ok()?;
        let ts = at.to_zoned(TimeZone::UTC).ok()?.timestamp();
        return Some((When::Timed { start: ts, end: ts }, Some("UTC".into())));
    }
    let at = DateTime::strptime("%Y%m%dT%H%M%S", value).ok()?;
    let (zone, name) = match param("TZID") {
        Some(id) => match lookup(id) {
            Some(zone) => (zone, Some(id.to_string())),
            None => (local.clone(), local.iana_name().map(str::to_string)),
        },
        None => (local.clone(), local.iana_name().map(str::to_string)),
    };
    let ts = zone.to_ambiguous_zoned(at).compatible().ok()?.timestamp();
    Some((When::Timed { start: ts, end: ts }, name))
}

/// `P1DT2H30M`, `PT45M`, `P1W` as a length.
fn parse_duration(value: &str) -> Option<SignedDuration> {
    let v = value.trim();
    let (negative, v) = match v.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, v.strip_prefix('+').unwrap_or(v)),
    };
    let v = v.strip_prefix('P')?;
    let mut secs: i64 = 0;
    let mut number = String::new();
    let mut in_time = false;
    for c in v.chars() {
        match c {
            'T' => in_time = true,
            '0'..='9' => number.push(c),
            unit => {
                let n: i64 = number.parse().ok()?;
                number.clear();
                secs += n * match (unit, in_time) {
                    ('W', false) => 7 * 86_400,
                    ('D', false) => 86_400,
                    ('H', true) => 3_600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
            }
        }
    }
    number
        .is_empty()
        .then_some(SignedDuration::from_secs(if negative {
            -secs
        } else {
            secs
        }))
}

/// Reads the events of an iCalendar file. `lookup` resolves `TZID` names; times in an unknown
/// zone, and floating times, are taken to be in `local`.
pub fn parse(text: &str, local: &TimeZone, lookup: &dyn Fn(&str) -> Option<TimeZone>) -> Parsed {
    let mut out = Parsed::default();
    let mut in_event = false;
    let mut alarm_depth = 0;
    let mut props: Vec<Prop> = Vec::new();
    for line in unfold(text) {
        let upper = line.trim().to_uppercase();
        match upper.as_str() {
            "BEGIN:VEVENT" => {
                in_event = true;
                alarm_depth = 0;
                props.clear();
                continue;
            }
            "BEGIN:VALARM" if in_event => {
                alarm_depth += 1;
                continue;
            }
            "END:VALARM" if in_event => {
                alarm_depth -= 1;
                continue;
            }
            "END:VEVENT" if in_event => {
                in_event = false;
                if out.events.len() >= MAX_EVENTS {
                    out.skipped += 1;
                } else {
                    match event_from(&props, local, lookup) {
                        Some(e) => out.events.push(e),
                        None => out.skipped += 1,
                    }
                }
                continue;
            }
            _ => {}
        }
        if in_event
            && alarm_depth == 0
            && let Some(p) = split_property(&line)
        {
            props.push(p);
        }
    }
    out
}

type Props = [Prop];

fn event_from(
    props: &Props,
    local: &TimeZone,
    lookup: &dyn Fn(&str) -> Option<TimeZone>,
) -> Option<Imported> {
    let get = |name: &str| props.iter().find(|(n, _, _)| n == name);
    if get("RECURRENCE-ID").is_some() {
        return None;
    }
    if get("STATUS").is_some_and(|(_, _, v)| v.trim().eq_ignore_ascii_case("CANCELLED")) {
        return None;
    }
    let (_, start_params, start_value) = get("DTSTART")?;
    let (mut when, time_zone) = parse_time(start_value, start_params, local, lookup)?;
    if let Some((_, end_params, end_value)) = get("DTEND")
        && let Some((end, _)) = parse_time(end_value, end_params, local, lookup)
    {
        when = match (when, end) {
            (When::AllDay { start, .. }, When::AllDay { start: end, .. }) if end > start => {
                When::AllDay { start, end }
            }
            (When::Timed { start, .. }, When::Timed { start: end, .. }) if end >= start => {
                When::Timed { start, end }
            }
            (other, _) => other,
        };
    } else if let Some((_, _, d)) = get("DURATION")
        && let Some(length) = parse_duration(d).filter(|d| d.as_secs() >= 0)
    {
        when = match when {
            When::Timed { start, .. } => When::Timed {
                start,
                end: start + length,
            },
            When::AllDay { start, .. } => {
                let days = ((length.as_secs() + 86_399) / 86_400).max(1) as i32;
                When::AllDay {
                    start,
                    end: crate::model::add_days(start, days),
                }
            }
        };
    } else if let When::Timed { start, .. } = when {
        // No end: an hour, the length Google uses.
        when = When::Timed {
            start,
            end: start + SignedDuration::from_hours(1),
        };
    }
    let text = |name: &str| {
        get(name)
            .map(|(_, _, v)| unescape(v).trim().to_string())
            .filter(|v| !v.is_empty())
    };
    let timed = matches!(when, When::Timed { .. });
    let time_zone = time_zone.filter(|_| timed);
    let rrule = get("RRULE")
        .map(|(_, _, v)| v.trim().to_string())
        .filter(|r| !r.is_empty() && time_zone.is_some() == timed);
    Some(Imported {
        title: text("SUMMARY").unwrap_or_else(|| "(No title)".into()),
        when,
        location: text("LOCATION"),
        description: text("DESCRIPTION"),
        rrule,
        time_zone,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn tokyo(name: &str) -> Option<TimeZone> {
        (name == "Asia/Tokyo").then(|| TimeZone::fixed(jiff::tz::offset(9)))
    }

    fn parse_utc(text: &str) -> Parsed {
        parse(text, &TimeZone::UTC, &tokyo)
    }

    #[test]
    fn reads_timed_all_day_and_folded_text() {
        let p = parse_utc(
            "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nSUMMARY:Planning\\, part 1\r\nDTSTART:20261125T090000Z\r\n\
             DTEND:20261125T100000Z\r\nLOCATION:Room 4\r\nDESCRIPTION:line one\\nline\r\n  two\r\nEND:VEVENT\r\n\
             BEGIN:VEVENT\r\nSUMMARY:Holiday\r\nDTSTART;VALUE=DATE:20261224\r\nDTEND;VALUE=DATE:20261227\r\nEND:VEVENT\r\n\
             END:VCALENDAR\r\n",
        );
        assert_eq!(p.skipped, 0);
        assert_eq!(p.events.len(), 2);
        let a = &p.events[0];
        assert_eq!(a.title, "Planning, part 1");
        assert_eq!(
            a.when,
            When::Timed {
                start: "2026-11-25T09:00:00Z".parse().unwrap(),
                end: "2026-11-25T10:00:00Z".parse().unwrap()
            }
        );
        assert_eq!(a.location.as_deref(), Some("Room 4"));
        assert_eq!(a.description.as_deref(), Some("line one\nline two"));
        assert_eq!(a.time_zone.as_deref(), Some("UTC"));
        assert_eq!(
            p.events[1].when,
            When::AllDay {
                start: date(2026, 12, 24),
                end: date(2026, 12, 27)
            }
        );
        assert_eq!(p.events[1].time_zone, None);
    }

    #[test]
    fn tzid_duration_and_recurrence() {
        let p = parse_utc(
            "BEGIN:VEVENT\nSUMMARY:Standup\nDTSTART;TZID=Asia/Tokyo:20261125T093000\nDURATION:PT45M\n\
             RRULE:FREQ=WEEKLY;BYDAY=WE\nEND:VEVENT\n",
        );
        let e = &p.events[0];
        assert_eq!(
            e.when,
            When::Timed {
                start: "2026-11-25T00:30:00Z".parse().unwrap(),
                end: "2026-11-25T01:15:00Z".parse().unwrap()
            }
        );
        assert_eq!(e.time_zone.as_deref(), Some("Asia/Tokyo"));
        assert_eq!(e.rrule.as_deref(), Some("FREQ=WEEKLY;BYDAY=WE"));
    }

    #[test]
    fn floating_times_use_the_local_zone_and_no_end_is_an_hour() {
        let p = parse_utc("BEGIN:VEVENT\nDTSTART:20261125T090000\nEND:VEVENT\n");
        let e = &p.events[0];
        assert_eq!(e.title, "(No title)");
        assert_eq!(
            e.when,
            When::Timed {
                start: "2026-11-25T09:00:00Z".parse().unwrap(),
                end: "2026-11-25T10:00:00Z".parse().unwrap()
            }
        );
        assert_eq!(e.time_zone.as_deref(), Some("UTC"));
    }

    #[test]
    fn cancelled_overrides_alarms_and_broken_entries_are_handled() {
        let p = parse_utc(
            "BEGIN:VEVENT\nSUMMARY:A\nSTATUS:CANCELLED\nDTSTART:20261125T090000Z\nEND:VEVENT\n\
             BEGIN:VEVENT\nSUMMARY:B\nRECURRENCE-ID:20261125T090000Z\nDTSTART:20261125T090000Z\nEND:VEVENT\n\
             BEGIN:VEVENT\nSUMMARY:C\nEND:VEVENT\n\
             BEGIN:VEVENT\nSUMMARY:D\nDTSTART:20261125T090000Z\nBEGIN:VALARM\nSUMMARY:not me\nEND:VALARM\nEND:VEVENT\n",
        );
        assert_eq!(p.skipped, 3);
        assert_eq!(p.events.len(), 1);
        assert_eq!(p.events[0].title, "D");
    }

    #[test]
    fn drafts_never_invite_anyone() {
        let p = parse_utc(
            "BEGIN:VEVENT\nSUMMARY:x\nDTSTART:20261125T090000Z\nATTENDEE:mailto:a@b.c\nEND:VEVENT\n",
        );
        let d = p.events[0].draft();
        assert!(d.guests.is_empty() && !d.meet && d.edit_guests.is_none());
        assert_eq!(d.title, "x");
    }

    #[test]
    fn durations() {
        assert_eq!(
            parse_duration("P1DT2H30M"),
            Some(SignedDuration::from_secs(86_400 + 9_000))
        );
        assert_eq!(
            parse_duration("P1W"),
            Some(SignedDuration::from_secs(604_800))
        );
        assert_eq!(parse_duration("PT"), Some(SignedDuration::ZERO));
        assert_eq!(parse_duration("1H"), None);
        assert_eq!(parse_duration("PT5"), None);
    }

    #[test]
    fn at_most_max_events_are_taken() {
        let one = "BEGIN:VEVENT\nSUMMARY:x\nDTSTART:20261125T090000Z\nEND:VEVENT\n";
        let p = parse_utc(&one.repeat(MAX_EVENTS + 3));
        assert_eq!((p.events.len(), p.skipped), (MAX_EVENTS, 3));
    }
}
