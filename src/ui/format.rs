//! Date and time strings for the UI.

use jiff::Zoned;
use jiff::civil::Date;
use jiff::tz::TimeZone;

use crate::config::ViewMode;
use crate::model;
use crate::model::{Event, When};

pub fn time(z: &Zoned, clock_24h: bool) -> String {
    if clock_24h {
        z.strftime("%H:%M").to_string()
    } else if z.minute() == 0 {
        z.strftime("%-I%P").to_string()
    } else {
        z.strftime("%-I:%M%P").to_string()
    }
}

/// "09:30 – 10:00", or "All day".
pub fn event_time(e: &Event, tz: &TimeZone, clock_24h: bool) -> String {
    match e.when {
        When::AllDay { .. } => "All day".into(),
        When::Timed { .. } => {
            let (s, en) = (e.start_zoned(tz), e.end_zoned(tz));
            if s == en {
                time(&s, clock_24h)
            } else {
                format!("{} – {}", time(&s, clock_24h), time(&en, clock_24h))
            }
        }
    }
}

/// Like [`event_time`], but from the point of view of `date`: an overnight event reads
/// "Until 00:30" on its second day, and "All day" on days it covers completely.
pub fn event_time_on(e: &Event, date: Date, tz: &TimeZone, clock_24h: bool) -> String {
    let When::Timed { .. } = e.when else {
        return "All day".into();
    };
    let (s, en) = (e.start_zoned(tz), e.end_zoned(tz));
    let ends_later = en.date() > date
        && !(en.date() == model::add_days(date, 1) && en.hour() == 0 && en.minute() == 0);
    match (s.date() < date, ends_later) {
        (false, false) => event_time(e, tz, clock_24h),
        (true, false) => format!("Until {}", time(&en, clock_24h)),
        (false, true) => format!("From {}", time(&s, clock_24h)),
        (true, true) => "All day".into(),
    }
}

/// [`event_time_on`] for a week/day block, then shorter forms to fall back on when it does
/// not fit: a same-day range drops to its start ("10:26am"). Mirrors [`event_time_on`].
pub fn event_time_options_on(e: &Event, date: Date, tz: &TimeZone, clock_24h: bool) -> Vec<String> {
    let full = event_time_on(e, date, tz, clock_24h);
    let When::Timed { .. } = e.when else {
        return vec![full];
    };
    let (s, en) = (e.start_zoned(tz), e.end_zoned(tz));
    let ends_later = en.date() > date
        && !(en.date() == model::add_days(date, 1) && en.hour() == 0 && en.minute() == 0);
    if s.date() < date || ends_later || s == en {
        vec![full]
    } else {
        vec![full, time(&s, clock_24h)]
    }
}

/// Agenda time column for `e` on `date`: (main line, optional second line), or None when
/// the row should read "All day". Mirrors [`event_time_on`].
pub fn event_time_parts_on(
    e: &Event,
    date: Date,
    tz: &TimeZone,
    clock_24h: bool,
) -> Option<(String, Option<String>)> {
    let When::Timed { .. } = e.when else {
        return None;
    };
    let (s, en) = (e.start_zoned(tz), e.end_zoned(tz));
    let ends_later = en.date() > date
        && !(en.date() == model::add_days(date, 1) && en.hour() == 0 && en.minute() == 0);
    match (s.date() < date, ends_later) {
        (false, false) if s == en => Some((time(&s, clock_24h), None)),
        (false, false) => Some((time(&s, clock_24h), Some(time(&en, clock_24h)))),
        (true, false) => Some((time(&en, clock_24h), Some("Ends".into()))),
        (false, true) => Some((time(&s, clock_24h), Some("Starts".into()))),
        (true, true) => None,
    }
}

/// Full description of when an event happens, for the detail panel.
pub fn event_when_long(e: &Event, tz: &TimeZone, clock_24h: bool) -> String {
    match e.when {
        When::AllDay { start, end } => {
            let last = model::add_days(end, -1);
            if last <= start {
                format!("{} · All day", start.strftime("%A, %-d %B %Y"))
            } else {
                format!(
                    "{} – {} · All day",
                    start.strftime("%a %-d %b"),
                    last.strftime("%a %-d %b %Y")
                )
            }
        }
        When::Timed { .. } => {
            let (s, en) = (e.start_zoned(tz), e.end_zoned(tz));
            if s.date() == en.date()
                || (en.date() == model::add_days(s.date(), 1) && en.hour() == 0 && en.minute() == 0)
            {
                format!(
                    "{} · {}",
                    s.strftime("%A, %-d %B %Y"),
                    event_time(e, tz, clock_24h)
                )
            } else {
                format!(
                    "{} {} – {} {}",
                    s.strftime("%a %-d %b"),
                    time(&s, clock_24h),
                    en.strftime("%a %-d %b"),
                    time(&en, clock_24h)
                )
            }
        }
    }
}

/// The header title as (strong, light): the bold part says what is shown, the light part when.
/// `compact` shortens only the day title's weekday ('Wed, 30 September'), keeping the comma and
/// full month of the long form so both tiers read alike; every other title and the year are
/// unchanged.
pub fn view_title_parts(view: ViewMode, anchor: Date, compact: bool) -> (String, String) {
    let year = |d: Date| d.strftime("%Y").to_string();
    match view {
        ViewMode::Month => (anchor.strftime("%B").to_string(), year(anchor)),
        ViewMode::Week => {
            let start = model::week_start(anchor);
            let end = model::add_days(start, 6);
            let strong = if start.month() == end.month() {
                format!("{} – {}", start.strftime("%-d"), end.strftime("%-d %B"))
            } else {
                format!("{} – {}", start.strftime("%-d %b"), end.strftime("%-d %b"))
            };
            let light = if start.year() == end.year() {
                year(end)
            } else {
                format!("{} – {}", year(start), year(end))
            };
            (strong, light)
        }
        ViewMode::Day => {
            let pattern = if compact { "%a, %-d %B" } else { "%A, %-d %B" };
            (anchor.strftime(pattern).to_string(), year(anchor))
        }
        ViewMode::Agenda => (format!("From {}", anchor.strftime("%-d %B")), year(anchor)),
    }
}

/// The gutter label for `hour`, split into the numeral and its AM/PM suffix.
pub fn hour_label_parts(hour: i8, clock_24h: bool) -> (String, Option<&'static str>) {
    if clock_24h {
        (format!("{hour:02}:00"), None)
    } else {
        match hour {
            0 => ("12".into(), Some("AM")),
            12 => ("12".into(), Some("PM")),
            h if h < 12 => (format!("{h}"), Some("AM")),
            h => (format!("{}", h - 12), Some("PM")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn titles() {
        let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            view_title_parts(ViewMode::Month, date(2026, 10, 21), false),
            pair("October", "2026")
        );
        assert_eq!(
            view_title_parts(ViewMode::Week, date(2026, 10, 21), false),
            pair("19 – 25 October", "2026")
        );
        assert_eq!(
            view_title_parts(ViewMode::Week, date(2026, 9, 30), false),
            pair("28 Sep – 4 Oct", "2026")
        );
        assert_eq!(
            view_title_parts(ViewMode::Week, date(2026, 12, 31), false),
            pair("28 Dec – 3 Jan", "2026 – 2027")
        );
        assert_eq!(
            view_title_parts(ViewMode::Day, date(2026, 10, 29), false),
            pair("Thursday, 29 October", "2026")
        );
        assert_eq!(
            view_title_parts(ViewMode::Agenda, date(2026, 10, 29), false),
            pair("From 29 October", "2026")
        );
    }

    #[test]
    fn compact_titles() {
        let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(
            view_title_parts(ViewMode::Day, date(2026, 9, 30), true),
            pair("Wed, 30 September", "2026")
        );
        assert_eq!(
            view_title_parts(ViewMode::Day, date(2026, 10, 29), true),
            pair("Thu, 29 October", "2026")
        );
        assert_eq!(
            view_title_parts(ViewMode::Week, date(2026, 12, 31), true),
            pair("28 Dec – 3 Jan", "2026 – 2027")
        );
        for (view, anchor) in [
            (ViewMode::Month, date(2026, 9, 30)),
            (ViewMode::Agenda, date(2026, 9, 30)),
            (ViewMode::Week, date(2026, 9, 30)),
            (ViewMode::Week, date(2026, 9, 23)),
        ] {
            assert_eq!(
                view_title_parts(view, anchor, true),
                view_title_parts(view, anchor, false)
            );
        }
    }

    #[test]
    fn overnight_labels() {
        let tz = TimeZone::UTC;
        let e = Event {
            id: "e".into(),
            calendar_id: "c".into(),
            title: "t".into(),
            when: When::Timed {
                start: "2026-10-21T23:00:00Z".parse().unwrap(),
                end: "2026-10-22T00:30:00Z".parse().unwrap(),
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
        };
        assert_eq!(
            event_time_on(&e, date(2026, 10, 21), &tz, true),
            "From 23:00"
        );
        assert_eq!(
            event_time_on(&e, date(2026, 10, 22), &tz, true),
            "Until 00:30"
        );
    }

    #[test]
    fn agenda_time_parts() {
        let tz = TimeZone::UTC;
        let timed = |start: &str, end: &str| Event {
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
        };
        let some = |a: &str, b: Option<&str>| Some((a.to_string(), b.map(String::from)));
        let d21 = date(2026, 10, 21);

        let e = timed("2026-10-21T09:30:00Z", "2026-10-21T10:00:00Z");
        assert_eq!(
            event_time_parts_on(&e, d21, &tz, true),
            some("09:30", Some("10:00"))
        );
        assert_eq!(
            event_time_parts_on(&e, d21, &tz, false),
            some("9:30am", Some("10am"))
        );

        let e = timed("2026-10-21T23:00:00Z", "2026-10-22T00:00:00Z");
        assert_eq!(
            event_time_parts_on(&e, d21, &tz, true),
            some("23:00", Some("00:00"))
        );

        let e = timed("2026-10-21T23:00:00Z", "2026-10-22T00:30:00Z");
        assert_eq!(
            event_time_parts_on(&e, d21, &tz, true),
            some("23:00", Some("Starts"))
        );
        assert_eq!(
            event_time_parts_on(&e, date(2026, 10, 22), &tz, true),
            some("00:30", Some("Ends"))
        );

        let e = timed("2026-10-21T09:00:00Z", "2026-10-23T17:00:00Z");
        assert_eq!(event_time_parts_on(&e, date(2026, 10, 22), &tz, true), None);

        let e = timed("2026-10-21T09:00:00Z", "2026-10-21T09:00:00Z");
        assert_eq!(event_time_parts_on(&e, d21, &tz, true), some("09:00", None));

        let mut e = e;
        e.when = When::AllDay {
            start: d21,
            end: date(2026, 10, 22),
        };
        assert_eq!(event_time_parts_on(&e, d21, &tz, true), None);
    }

    #[test]
    fn block_time_options() {
        let tz = TimeZone::UTC;
        let timed = |start: &str, end: &str| Event {
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
        };
        let d21 = date(2026, 10, 21);
        let d22 = date(2026, 10, 22);

        let e = timed("2026-10-21T10:26:00Z", "2026-10-21T11:11:00Z");
        assert_eq!(
            event_time_options_on(&e, d21, &tz, false),
            ["10:26am – 11:11am", "10:26am"]
        );
        assert_eq!(
            event_time_options_on(&e, d21, &tz, true),
            ["10:26 – 11:11", "10:26"]
        );

        let e = timed("2026-10-21T10:00:00Z", "2026-10-21T11:30:00Z");
        assert_eq!(
            event_time_options_on(&e, d21, &tz, false),
            ["10am – 11:30am", "10am"]
        );
        assert_eq!(
            event_time_options_on(&e, d21, &tz, true),
            ["10:00 – 11:30", "10:00"]
        );

        let e = timed("2026-10-21T22:30:00Z", "2026-10-22T11:00:00Z");
        assert_eq!(event_time_options_on(&e, d21, &tz, true), ["From 22:30"]);
        assert_eq!(event_time_options_on(&e, d21, &tz, false), ["From 10:30pm"]);
        assert_eq!(event_time_options_on(&e, d22, &tz, true), ["Until 11:00"]);
        assert_eq!(event_time_options_on(&e, d22, &tz, false), ["Until 11am"]);

        // Ending at midnight still reads as a same-day range.
        let e = timed("2026-10-21T23:00:00Z", "2026-10-22T00:00:00Z");
        assert_eq!(
            event_time_options_on(&e, d21, &tz, true),
            ["23:00 – 00:00", "23:00"]
        );

        let e = timed("2026-10-21T09:00:00Z", "2026-10-21T09:00:00Z");
        assert_eq!(event_time_options_on(&e, d21, &tz, true), ["09:00"]);

        let mut e = e;
        e.when = When::AllDay {
            start: d21,
            end: d22,
        };
        assert_eq!(event_time_options_on(&e, d21, &tz, true), ["All day"]);
    }

    #[test]
    fn twelve_hour_times() {
        let tz = TimeZone::UTC;
        let z = |s: &str| s.parse::<jiff::Timestamp>().unwrap().to_zoned(tz.clone());
        assert_eq!(time(&z("2026-10-21T09:30:00Z"), false), "9:30am");
        assert_eq!(time(&z("2026-10-21T15:00:00Z"), false), "3pm");
        assert_eq!(time(&z("2026-10-21T15:00:00Z"), true), "15:00");
        assert_eq!(hour_label_parts(0, false), (String::from("12"), Some("AM")));
        assert_eq!(hour_label_parts(13, false), (String::from("1"), Some("PM")));
    }

    #[test]
    fn hour_label_parts_modes() {
        let twelve = [
            (0, "12", "AM"),
            (9, "9", "AM"),
            (12, "12", "PM"),
            (13, "1", "PM"),
        ];
        for (h, num, suffix) in twelve {
            assert_eq!(
                hour_label_parts(h, false),
                (String::from(num), Some(suffix))
            );
        }
        let full = [(0, "00:00"), (9, "09:00"), (12, "12:00"), (13, "13:00")];
        for (h, num) in full {
            assert_eq!(hour_label_parts(h, true), (String::from(num), None));
        }
    }
}
