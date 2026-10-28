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

pub fn view_title(view: ViewMode, anchor: Date) -> String {
    match view {
        ViewMode::Month => anchor.strftime("%B %Y").to_string(),
        ViewMode::Week => {
            let start = model::week_start(anchor);
            let end = model::add_days(start, 6);
            if start.month() == end.month() {
                format!("{} – {}", start.strftime("%-d"), end.strftime("%-d %B %Y"))
            } else if start.year() == end.year() {
                format!(
                    "{} – {}",
                    start.strftime("%-d %b"),
                    end.strftime("%-d %b %Y")
                )
            } else {
                format!(
                    "{} – {}",
                    start.strftime("%-d %b %Y"),
                    end.strftime("%-d %b %Y")
                )
            }
        }
        ViewMode::Day => anchor.strftime("%A, %-d %B %Y").to_string(),
        ViewMode::Agenda => format!("From {}", anchor.strftime("%-d %B %Y")),
    }
}

pub fn hour_label(hour: i8, clock_24h: bool) -> String {
    if clock_24h {
        format!("{hour:02}:00")
    } else {
        match hour {
            0 => "12am".into(),
            12 => "12pm".into(),
            h if h < 12 => format!("{h}am"),
            h => format!("{}pm", h - 12),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn titles() {
        assert_eq!(
            view_title(ViewMode::Month, date(2026, 10, 21)),
            "October 2026"
        );
        assert_eq!(
            view_title(ViewMode::Week, date(2026, 10, 21)),
            "19 – 25 October 2026"
        );
        assert_eq!(
            view_title(ViewMode::Week, date(2026, 9, 30)),
            "28 Sep – 4 Oct 2026"
        );
        assert_eq!(
            view_title(ViewMode::Week, date(2026, 12, 31)),
            "28 Dec 2026 – 3 Jan 2027"
        );
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
    fn twelve_hour_times() {
        let tz = TimeZone::UTC;
        let z = |s: &str| s.parse::<jiff::Timestamp>().unwrap().to_zoned(tz.clone());
        assert_eq!(time(&z("2026-10-21T09:30:00Z"), false), "9:30am");
        assert_eq!(time(&z("2026-10-21T15:00:00Z"), false), "3pm");
        assert_eq!(time(&z("2026-10-21T15:00:00Z"), true), "15:00");
        assert_eq!(hour_label(0, false), "12am");
        assert_eq!(hour_label(13, false), "1pm");
    }
}
