//! Secondary time zones shown beside the hour gutter of the week and day views.

use jiff::civil::Date;
use jiff::tz::TimeZone;

/// How many extra zones fit beside the local gutter.
pub const MAX_ZONES: usize = 2;

/// Reads the settings field: IANA names separated by commas or semicolons. `lookup` resolves a
/// name (the real database in the app; a stand-in in tests).
pub fn parse_zone_list(
    text: &str,
    lookup: impl Fn(&str) -> Option<TimeZone>,
) -> Result<Vec<String>, String> {
    let mut zones: Vec<String> = vec![];
    for name in text
        .split([',', ';'])
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        if lookup(name).is_none() {
            return Err(format!(
                "\"{name}\" is not a time zone name. Use names like Europe/London or Asia/Tokyo."
            ));
        }
        if !zones.iter().any(|z| z == name) {
            zones.push(name.to_string());
        }
    }
    if zones.len() > MAX_ZONES {
        return Err(format!("At most {MAX_ZONES} extra time zones fit."));
    }
    Ok(zones)
}

/// The system database lookup used by the app.
pub fn lookup(name: &str) -> Option<TimeZone> {
    TimeZone::get(name).ok()
}

/// A zone's gutter header: its abbreviation on `date` (`JST`, `EST`), else the last part of its
/// name (`New York`).
pub fn zone_label(name: &str, zone: &TimeZone, date: Date) -> String {
    let abbreviation = zone
        .to_timestamp(date.at(12, 0, 0, 0))
        .ok()
        .map(|t| t.to_zoned(zone.clone()).strftime("%Z").to_string())
        // A zone without an abbreviation shows its offset ("+03"), which reads fine too.
        .filter(|a| !a.is_empty());
    abbreviation.unwrap_or_else(|| name.rsplit('/').next().unwrap_or(name).replace('_', " "))
}

/// The label for the gutter row that is `hour` o'clock locally on `date`, in `zone`: the numeral
/// and, on a 12-hour clock, its AM/PM suffix. Minutes show only for zones off the hour.
pub fn hour_label(
    hour: i8,
    date: Date,
    local: &TimeZone,
    zone: &TimeZone,
    clock_24h: bool,
) -> (String, Option<&'static str>) {
    let Ok(at) = local.to_timestamp(date.at(hour, 0, 0, 0)) else {
        return (String::new(), None);
    };
    let there = at.to_zoned(zone.clone());
    let (h, m) = (there.hour(), there.minute());
    if clock_24h {
        return (format!("{h:02}:{m:02}"), None);
    }
    let suffix = if h < 12 { "AM" } else { "PM" };
    let h12 = match h % 12 {
        0 => 12,
        n => n,
    };
    let num = if m == 0 {
        format!("{h12}")
    } else {
        format!("{h12}:{m:02}")
    };
    (num, Some(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;
    use jiff::tz::{Offset, TimeZone};

    fn fixed(hours: i8, minutes: i32) -> TimeZone {
        TimeZone::fixed(Offset::from_seconds(hours as i32 * 3600 + minutes * 60).unwrap())
    }

    #[test]
    fn the_zone_list_is_checked_deduplicated_and_capped() {
        let known = |n: &str| {
            matches!(n, "Europe/London" | "Asia/Tokyo" | "Asia/Kolkata").then(|| TimeZone::UTC)
        };
        assert_eq!(
            parse_zone_list(" Europe/London ; Asia/Tokyo, Europe/London", known),
            Ok(vec!["Europe/London".into(), "Asia/Tokyo".into()])
        );
        assert_eq!(parse_zone_list("", known), Ok(vec![]));
        assert!(
            parse_zone_list("Mars/Olympus", known)
                .unwrap_err()
                .contains("Mars/Olympus")
        );
        assert!(
            parse_zone_list("Europe/London, Asia/Tokyo, Asia/Kolkata", known)
                .unwrap_err()
                .contains("At most")
        );
    }

    #[test]
    fn hour_labels_show_what_the_clock_reads_in_the_other_zone() {
        let local = fixed(0, 0);
        let d = date(2026, 11, 25);
        // 9 o'clock here is 18:00 in Tokyo (+9) and 14:30 in Kolkata (+5:30).
        assert_eq!(
            hour_label(9, d, &local, &fixed(9, 0), true),
            ("18:00".into(), None)
        );
        assert_eq!(
            hour_label(9, d, &local, &fixed(5, 30), true),
            ("14:30".into(), None)
        );
        assert_eq!(
            hour_label(9, d, &local, &fixed(9, 0), false),
            ("6".into(), Some("PM"))
        );
        assert_eq!(
            hour_label(9, d, &local, &fixed(5, 30), false),
            ("2:30".into(), Some("PM"))
        );
        // Midnight and noon read 12, and the day rolls over.
        assert_eq!(
            hour_label(3, d, &local, &fixed(9, 0), false),
            ("12".into(), Some("PM"))
        );
        assert_eq!(
            hour_label(15, d, &local, &fixed(9, 0), false),
            ("12".into(), Some("AM"))
        );
        assert_eq!(
            hour_label(15, d, &local, &fixed(-5, 0), true),
            ("10:00".into(), None)
        );
    }
}
