//! Deterministic natural-language event entry ("Lunch with Sam tomorrow 1pm at Dishoom").
//!
//! No network, no LLM: the parser is a pure function of the input text and `now`, so it runs on
//! every keystroke and can be tested exhaustively. English only.
//!
//! Recognised, in any order (the rest becomes the title):
//! - dates: `today`, `tomorrow`/`tmr`, `monday`..`sun`, `next fri`, `this fri`, `in 3 days`,
//!   `in 2 weeks`, `jan 5`, `5 january 2027`, `2026-12-01`
//! - times: `1pm`, `1:30pm`, `13:00`, `noon`, `midnight`, `at 5`, ranges (`1-2pm`, `9 to 10:30`),
//!   relative (`in 2 hours`, `in 45 min`)
//! - durations: `for 2h`, `for 90 minutes`, `1.5h`
//! - `all day`
//! - location: everything after `at ...` / `@ ...` that is not a time
//! - guests: anything shaped like an email address
//! - calendar: `/work` (matched against calendar names by the caller)
//! - video call: `+meet`
//! - recurrence: `every day|weekday|week|month|year|<weekday>`, `daily`, `weekly`, `monthly`
//!
//! Numeric dates like `12/25` are deliberately unsupported: month/day order is locale-dependent
//! and guessing wrong silently books the wrong day.

use jiff::civil::{Date, DateTime, Time, Weekday};
use jiff::{ToSpan, Zoned};

use crate::model::When;

/// Default length of a timed event with no stated end.
pub const DEFAULT_DURATION_MINUTES: i64 = 60;

/// Recurrence recognised in the text, expressed as a Google `RRULE`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recurrence {
    pub rrule: String,
    /// Human label for the live preview.
    pub label: String,
}

/// The fields found in the input.
#[derive(Clone, Debug, PartialEq)]
pub struct Parsed {
    pub title: String,
    /// `None` when the text names neither a date nor a time; the caller picks a default slot.
    pub when: Option<When>,
    pub location: Option<String>,
    pub guests: Vec<String>,
    /// Text after `/`, to be matched against calendar names by the caller.
    pub calendar_hint: Option<String>,
    pub meet: bool,
    pub recurrence: Option<Recurrence>,
    /// The text starts with `todo` or `task:`: it is a Google Task, not an event.
    pub task: bool,
}

struct Tok {
    text: String,
    lower: String,
    used: bool,
}

pub fn parse(input: &str, now: &Zoned) -> Parsed {
    let mut toks: Vec<Tok> = input
        .split_whitespace()
        .map(|t| Tok {
            text: t.to_string(),
            lower: t.to_lowercase(),
            used: false,
        })
        .collect();
    let tz = now.time_zone().clone();
    let today = now.date();

    let mut out = Parsed {
        title: String::new(),
        when: None,
        location: None,
        guests: vec![],
        calendar_hint: None,
        meet: false,
        recurrence: None,
        task: false,
    };

    // A leading `todo` (or `todo:`, `task:`) makes it a task. A bare `task` is left alone: it
    // starts plenty of event titles ("Task force sync").
    if toks.len() > 1 && matches!(toks[0].lower.as_str(), "todo" | "todo:" | "task:") {
        toks[0].used = true;
        out.task = true;
    }

    // Flags and tags.
    for t in toks.iter_mut() {
        if t.lower == "+meet" || t.lower == "+video" {
            t.used = true;
            out.meet = true;
        } else if is_email(&t.text) {
            t.used = true;
            out.guests
                .push(t.text.trim_matches(|c| ",;".contains(c)).to_string());
        } else if let Some(name) = t.text.strip_prefix('/')
            && !name.is_empty()
            && out.calendar_hint.is_none()
        {
            t.used = true;
            out.calendar_hint = Some(name.to_string());
        }
    }

    out.recurrence = take_recurrence(&mut toks);
    let all_day = take_phrase(&mut toks, &["all", "day"]) | take_phrase(&mut toks, &["all-day"]);
    let duration = take_duration(&mut toks);
    let relative = take_relative(&mut toks);
    let clock = if relative.is_none() {
        take_clock(&mut toks)
    } else {
        None
    };
    let date = take_date(&mut toks, today);

    // Location: tokens after the first remaining `at` / `@`, up to the next consumed token.
    if let Some(i) = toks
        .iter()
        .position(|t| !t.used && (t.lower == "at" || t.lower == "@"))
    {
        let loc: Vec<usize> = (i + 1..toks.len()).take_while(|&j| !toks[j].used).collect();
        if !loc.is_empty() {
            out.location = Some(
                loc.iter()
                    .map(|&j| toks[j].text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            toks[i].used = true;
            for j in loc {
                toks[j].used = true;
            }
        }
    }

    out.title = toks
        .iter()
        .filter(|t| !t.used)
        .map(|t| t.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    out.title = trim_connectors(&out.title);

    out.when = build_when(now, &tz, date, clock, relative, duration, all_day);
    out
}

/// Whether `s` looks like an email address.
fn is_email(s: &str) -> bool {
    let s = s.trim_matches(|c| ",;".contains(c));
    match s.split_once('@') {
        Some((user, host)) => {
            !user.is_empty()
                && host.contains('.')
                && !host.starts_with('.')
                && !host.ends_with('.')
                && !s.contains(char::is_whitespace)
        }
        None => false,
    }
}

/// Drops dangling `on`/`at`/`for`/`from`/`by`/`in` left where a date or time was removed.
fn trim_connectors(title: &str) -> String {
    const CONNECTORS: [&str; 7] = ["on", "at", "for", "from", "by", "in", "to"];
    let mut words: Vec<&str> = title.split_whitespace().collect();
    // A trailing "with"/"and" is what is left when a guest's email was lifted out of the title.
    let dangling = |w: &str| CONNECTORS.contains(&w) || matches!(w, "with" | "and" | "w/");
    while words
        .last()
        .is_some_and(|w| dangling(w.to_lowercase().as_str()))
    {
        words.pop();
    }
    while words
        .first()
        .is_some_and(|w| CONNECTORS.contains(&w.to_lowercase().as_str()))
        && words.len() > 1
    {
        words.remove(0);
    }
    words.join(" ")
}

// ----- when ---------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Clock {
    start: Time,
    end: Option<Time>,
}

fn build_when(
    now: &Zoned,
    tz: &jiff::tz::TimeZone,
    date: Option<Date>,
    clock: Option<Clock>,
    relative: Option<i64>,
    duration: Option<i64>,
    all_day: bool,
) -> Option<When> {
    if let Some(minutes) = relative {
        // "in 2 hours" starts then; it lasts the stated duration or the default.
        let start = now.timestamp().checked_add((minutes * 60).seconds()).ok()?;
        let length = duration.unwrap_or(DEFAULT_DURATION_MINUTES);
        let end = start.checked_add((length * 60).seconds()).ok()?;
        return Some(When::Timed { start, end });
    }
    match (date, clock) {
        (None, None) => all_day.then(|| {
            let d = now.date();
            When::AllDay {
                start: d,
                end: d.tomorrow().unwrap_or(d),
            }
        }),
        (Some(d), None) => {
            // A date with no time is all-day; a duration without a time has nothing to anchor to.
            Some(When::AllDay {
                start: d,
                end: d.tomorrow().unwrap_or(d),
            })
        }
        (_, Some(_)) if all_day => {
            let d = date.unwrap_or(now.date());
            Some(When::AllDay {
                start: d,
                end: d.tomorrow().unwrap_or(d),
            })
        }
        (d, Some(c)) => {
            let day = d.unwrap_or_else(|| {
                // No date: today if the time is still ahead, otherwise tomorrow.
                if DateTime::from_parts(now.date(), c.start) > now.datetime() {
                    now.date()
                } else {
                    now.date().tomorrow().unwrap_or(now.date())
                }
            });
            let start = day.to_datetime(c.start).to_zoned(tz.clone()).ok()?;
            let end = match (c.end, duration) {
                (Some(e), _) => {
                    let mut end_day = day;
                    if e <= c.start {
                        end_day = day.tomorrow().unwrap_or(day);
                    }
                    end_day.to_datetime(e).to_zoned(tz.clone()).ok()?
                }
                (None, Some(m)) => start.checked_add((m.max(0)).minutes()).ok()?,
                (None, None) => start.checked_add(DEFAULT_DURATION_MINUTES.minutes()).ok()?,
            };
            Some(When::Timed {
                start: start.timestamp(),
                end: end.timestamp(),
            })
        }
    }
}

// ----- token passes -------------------------------------------------------------------------

/// Marks `words` (consecutive, case-insensitive) used when found; returns whether they were.
fn take_phrase(toks: &mut [Tok], words: &[&str]) -> bool {
    if toks.len() < words.len() {
        return false;
    }
    for i in 0..=toks.len() - words.len() {
        if (0..words.len()).all(|k| !toks[i + k].used && toks[i + k].lower == words[k]) {
            for k in 0..words.len() {
                toks[i + k].used = true;
            }
            return true;
        }
    }
    false
}

/// `every day`, `daily`, `every monday`, ...
fn take_recurrence(toks: &mut [Tok]) -> Option<Recurrence> {
    for i in 0..toks.len() {
        if toks[i].used {
            continue;
        }
        let single = match toks[i].lower.as_str() {
            "daily" => Some(("FREQ=DAILY".to_string(), "Daily".to_string())),
            "weekly" => Some(("FREQ=WEEKLY".to_string(), "Weekly".to_string())),
            "monthly" => Some(("FREQ=MONTHLY".to_string(), "Monthly".to_string())),
            "yearly" | "annually" => Some(("FREQ=YEARLY".to_string(), "Yearly".to_string())),
            "weekdays" => Some((
                "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR".to_string(),
                "Every weekday".to_string(),
            )),
            _ => None,
        };
        if let Some((rrule, label)) = single {
            toks[i].used = true;
            return Some(Recurrence { rrule, label });
        }
        if toks[i].lower == "every" && i + 1 < toks.len() && !toks[i + 1].used {
            let next = toks[i + 1].lower.clone();
            let found = match next.as_str() {
                "day" => Some(("FREQ=DAILY".to_string(), "Daily".to_string())),
                "week" => Some(("FREQ=WEEKLY".to_string(), "Weekly".to_string())),
                "month" => Some(("FREQ=MONTHLY".to_string(), "Monthly".to_string())),
                "year" => Some(("FREQ=YEARLY".to_string(), "Yearly".to_string())),
                "weekday" => Some((
                    "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR".to_string(),
                    "Every weekday".to_string(),
                )),
                other => weekday_of(other).map(|w| {
                    (
                        format!("FREQ=WEEKLY;BYDAY={}", byday(w)),
                        format!("Every {}", weekday_name(w)),
                    )
                }),
            };
            if let Some((rrule, label)) = found {
                toks[i].used = true;
                toks[i + 1].used = true;
                return Some(Recurrence { rrule, label });
            }
        }
    }
    None
}

/// `for 2h`, `for 90 min`, `for 1.5 hours`, or a bare `45min` / `2h`.
fn take_duration(toks: &mut [Tok]) -> Option<i64> {
    for i in 0..toks.len() {
        if toks[i].used {
            continue;
        }
        let has_for = toks[i].lower == "for";
        let j = if has_for { i + 1 } else { i };
        if j >= toks.len() || toks[j].used {
            continue;
        }
        // "2h", "90min", "1.5h" in one token, or a number followed by a unit token.
        if let Some(m) = duration_token(&toks[j].lower) {
            toks[j].used = true;
            if has_for {
                toks[i].used = true;
            }
            return Some(m);
        }
        if let Ok(n) = toks[j].lower.parse::<f64>()
            && j + 1 < toks.len()
            && !toks[j + 1].used
            && let Some(unit) = unit_minutes(&toks[j + 1].lower)
            && has_for
        {
            toks[i].used = true;
            toks[j].used = true;
            toks[j + 1].used = true;
            return Some((n * unit as f64).round() as i64);
        }
    }
    None
}

fn duration_token(s: &str) -> Option<i64> {
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
    let (num, unit) = s.split_at(split);
    if num.is_empty() {
        return None;
    }
    let n: f64 = num.parse().ok()?;
    let unit = unit_minutes(unit)?;
    Some((n * unit as f64).round() as i64)
}

fn unit_minutes(u: &str) -> Option<i64> {
    match u {
        "m" | "min" | "mins" | "minute" | "minutes" => Some(1),
        "h" | "hr" | "hrs" | "hour" | "hours" => Some(60),
        _ => None,
    }
}

/// `in 2 hours`, `in 45 min`. Returns minutes from now. (`in 3 days` is a date, see below.)
fn take_relative(toks: &mut [Tok]) -> Option<i64> {
    for i in 0..toks.len().saturating_sub(2) {
        if toks[i].used || toks[i].lower != "in" {
            continue;
        }
        let (a, b) = (&toks[i + 1], &toks[i + 2]);
        if a.used || b.used {
            continue;
        }
        if let (Ok(n), Some(unit)) = (a.lower.parse::<f64>(), unit_minutes(&b.lower)) {
            for k in 0..3 {
                toks[i + k].used = true;
            }
            return Some((n * unit as f64).round() as i64);
        }
    }
    None
}

/// A time of day or a time range.
fn take_clock(toks: &mut [Tok]) -> Option<Clock> {
    let n = toks.len();
    for i in 0..n {
        if toks[i].used {
            continue;
        }
        // Optional lead-in: "at", "from", "@".
        let lead = matches!(toks[i].lower.as_str(), "at" | "from" | "@");
        let s = if lead { i + 1 } else { i };
        if s >= n || toks[s].used {
            continue;
        }
        let Some((start, start_meridiem, mut end_idx)) = parse_time_at(toks, s, lead) else {
            continue;
        };
        // Range: "-", "to", "until", "till" or "start-end" in one token.
        let mut end: Option<(Time, Option<bool>)> = None;
        if let Some(e) = range_end_in_token(&toks[s].lower, lead) {
            end = Some(e);
        } else if end_idx < n && !toks[end_idx].used {
            let w = toks[end_idx].lower.as_str();
            let (skip, ok) = match w {
                "to" | "until" | "till" | "-" | "–" => (1, true),
                _ => (0, false),
            };
            if ok
                && end_idx + skip < n
                && !toks[end_idx + skip].used
                && let Some((t, m, after)) = parse_time_at(toks, end_idx + skip, true)
            {
                end = Some((t, m));
                for t in &mut toks[end_idx..after] {
                    t.used = true;
                }
                end_idx = after;
            }
        }
        let consumed_to = if end.is_some() { s.max(end_idx) } else { s };
        for t in &mut toks[i..consumed_to] {
            t.used = true;
        }
        toks[s].used = true;
        if let Some(extra) = meridiem_split_len(toks, s) {
            for t in &mut toks[s..s + extra] {
                t.used = true;
            }
        }
        let (start, end) = resolve_range(start, start_meridiem, end);
        return Some(Clock { start, end });
    }
    None
}

/// For a token like `1pm`, `1:30pm`, `13:00`, `noon` — or `1` followed by an `am`/`pm` token —
/// returns the time, whether the meridiem was explicit (`Some(is_pm)`), and the index after it.
/// A bare number is only a time when `bare_ok` (i.e. it followed `at`/`from`).
fn parse_time_at(toks: &[Tok], i: usize, bare_ok: bool) -> Option<(Time, Option<bool>, usize)> {
    let t = &toks[i].lower;
    // Single token possibly containing a range ("1-2pm"): parse only the head here.
    let head = t.split(['-', '–']).next().unwrap_or(t);
    match head {
        "noon" => return Some((Time::constant(12, 0, 0, 0), Some(true), i + 1)),
        "midnight" => return Some((Time::constant(0, 0, 0, 0), Some(false), i + 1)),
        _ => {}
    }
    let (digits, suffix) = split_suffix(head);
    let (h, m) = parse_hm(digits)?;
    let mut meridiem = match suffix {
        "am" | "a" => Some(false),
        "pm" | "p" => Some(true),
        "" => None,
        _ => return None,
    };
    let mut next = i + 1;
    if meridiem.is_none()
        && head == t.as_str()
        && let Some(n) = toks.get(i + 1)
        && !n.used
    {
        if n.lower == "am" {
            meridiem = Some(false);
            next += 1;
        } else if n.lower == "pm" {
            meridiem = Some(true);
            next += 1;
        }
    }
    let has_colon = digits.contains(':') || digits.contains('.');
    // "1-2pm" and "9-10:30" are unambiguous ranges even without "at".
    let bare_ok = bare_ok
        || t.split_once(['-', '–']).is_some_and(|(_, tail)| {
            tail.contains(':') || tail.ends_with("am") || tail.ends_with("pm")
        });
    // Without am/pm or a colon it is only a time after "at"/"from".
    if meridiem.is_none() && !has_colon && !bare_ok {
        return None;
    }
    // "24h" style needs a colon, so "13" alone after "at" is accepted but bare "13" is not.
    let hour24 = match meridiem {
        Some(pm) => {
            if !(1..=12).contains(&h) {
                return None;
            }
            to_24h(h, pm)
        }
        None => {
            if h > 23 || m > 59 {
                return None;
            }
            if has_colon || h > 12 || h == 0 {
                h
            } else {
                // Bare "at 5": business-hours guess — 1-6 afternoon, 7-11 morning, 12 noon.
                to_24h(h, (1..=6).contains(&h) || h == 12)
            }
        }
    };
    if m > 59 {
        return None;
    }
    Some((Time::constant(hour24 as i8, m as i8, 0, 0), meridiem, next))
}

fn meridiem_split_len(toks: &[Tok], s: usize) -> Option<usize> {
    let t = &toks[s].lower;
    let head = t.split(['-', '–']).next().unwrap_or(t);
    let (_, suffix) = split_suffix(head);
    if suffix.is_empty()
        && head == t.as_str()
        && toks
            .get(s + 1)
            .is_some_and(|n| n.lower == "am" || n.lower == "pm")
    {
        Some(2)
    } else {
        None
    }
}

/// `1-2pm` / `9:30-10:30` inside one token: the part after the dash.
fn range_end_in_token(t: &str, _lead: bool) -> Option<(Time, Option<bool>)> {
    let (_, tail) = t.split_once(['-', '–'])?;
    let (digits, suffix) = split_suffix(tail);
    let (h, m) = parse_hm(digits)?;
    let meridiem = match suffix {
        "am" | "a" => Some(false),
        "pm" | "p" => Some(true),
        "" => None,
        _ => return None,
    };
    let hour24 = match meridiem {
        Some(pm) if (1..=12).contains(&h) => to_24h(h, pm),
        Some(_) => return None,
        None if h <= 23 => h,
        None => return None,
    };
    (m <= 59).then(|| (Time::constant(hour24 as i8, m as i8, 0, 0), meridiem))
}

fn split_suffix(s: &str) -> (&str, &str) {
    let i = s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len());
    s.split_at(i)
}

fn parse_hm(s: &str) -> Option<(u32, u32)> {
    if s.is_empty() {
        return None;
    }
    let (h, m) = match s.split_once([':', '.']) {
        Some((h, m)) if m.len() == 2 => (h, m),
        Some(_) => return None,
        None => (s, "0"),
    };
    Some((h.parse().ok()?, m.parse().ok()?))
}

fn to_24h(h: u32, pm: bool) -> u32 {
    match (h % 12, pm) {
        (x, true) => x + 12,
        (x, false) => x,
    }
}

/// Applies the end's meridiem to a start with none: `1-2pm` is 1pm-2pm, `11-1pm` is 11am-1pm.
fn resolve_range(
    start: Time,
    start_meridiem: Option<bool>,
    end: Option<(Time, Option<bool>)>,
) -> (Time, Option<Time>) {
    let Some((end_t, end_m)) = end else {
        return (start, None);
    };
    let mut start = start;
    if start_meridiem.is_none()
        && let Some(pm) = end_m
        && start.hour() <= 12
    {
        // Pick the reading of the start that is not after the end.
        let as_pm = Time::constant(
            start.hour() % 12 + if pm { 12 } else { 0 },
            start.minute(),
            0,
            0,
        );
        let as_am = Time::constant(start.hour() % 12, start.minute(), 0, 0);
        start = if as_pm <= end_t { as_pm } else { as_am };
    }
    (start, Some(end_t))
}

// ----- dates --------------------------------------------------------------------------------

const MONTHS: [(&str, i8); 12] = [
    ("jan", 1),
    ("feb", 2),
    ("mar", 3),
    ("apr", 4),
    ("may", 5),
    ("jun", 6),
    ("jul", 7),
    ("aug", 8),
    ("sep", 9),
    ("oct", 10),
    ("nov", 11),
    ("dec", 12),
];

fn month_of(s: &str) -> Option<i8> {
    let s = s.trim_end_matches('.');
    MONTHS
        .iter()
        .find(|(abbr, _)| s == *abbr || (s.len() > 3 && full_month(abbr, s)))
        .map(|(_, n)| *n)
}

fn full_month(abbr: &str, s: &str) -> bool {
    let full = match abbr {
        "jan" => "january",
        "feb" => "february",
        "mar" => "march",
        "apr" => "april",
        "may" => "may",
        "jun" => "june",
        "jul" => "july",
        "aug" => "august",
        "sep" => "september",
        "oct" => "october",
        "nov" => "november",
        "dec" => "december",
        _ => return false,
    };
    s == full || (abbr == "sep" && s == "sept")
}

fn weekday_of(s: &str) -> Option<Weekday> {
    Some(match s {
        "mon" | "monday" => Weekday::Monday,
        "tue" | "tues" | "tuesday" => Weekday::Tuesday,
        "wed" | "weds" | "wednesday" => Weekday::Wednesday,
        "thu" | "thur" | "thurs" | "thursday" => Weekday::Thursday,
        "fri" | "friday" => Weekday::Friday,
        "sat" | "saturday" => Weekday::Saturday,
        "sun" | "sunday" => Weekday::Sunday,
        _ => return None,
    })
}

fn byday(w: Weekday) -> &'static str {
    match w {
        Weekday::Monday => "MO",
        Weekday::Tuesday => "TU",
        Weekday::Wednesday => "WE",
        Weekday::Thursday => "TH",
        Weekday::Friday => "FR",
        Weekday::Saturday => "SA",
        Weekday::Sunday => "SU",
    }
}

fn weekday_name(w: Weekday) -> &'static str {
    match w {
        Weekday::Monday => "Monday",
        Weekday::Tuesday => "Tuesday",
        Weekday::Wednesday => "Wednesday",
        Weekday::Thursday => "Thursday",
        Weekday::Friday => "Friday",
        Weekday::Saturday => "Saturday",
        Weekday::Sunday => "Sunday",
    }
}

fn day_number(s: &str) -> Option<i8> {
    let s = s
        .trim_end_matches("st")
        .trim_end_matches("nd")
        .trim_end_matches("rd")
        .trim_end_matches("th");
    let n: i8 = s.parse().ok()?;
    (1..=31).contains(&n).then_some(n)
}

fn take_date(toks: &mut [Tok], today: Date) -> Option<Date> {
    let n = toks.len();
    for i in 0..n {
        if toks[i].used {
            continue;
        }
        let w = toks[i].lower.clone();
        // Optional "on" lead-in is consumed with the date.
        let lead = |toks: &mut [Tok], k: usize| {
            if i > 0 && !toks[i - 1].used && toks[i - 1].lower == "on" {
                toks[i - 1].used = true;
            }
            for j in 0..k {
                toks[i + j].used = true;
            }
        };

        match w.as_str() {
            "today" | "tonight" => {
                lead(toks, 1);
                return Some(today);
            }
            "tomorrow" | "tmr" | "tmrw" | "tom" => {
                lead(toks, 1);
                return today.tomorrow().ok();
            }
            _ => {}
        }

        // ISO date.
        if let Ok(d) = w.parse::<Date>() {
            lead(toks, 1);
            return Some(d);
        }

        // next/this <weekday>
        if (w == "next" || w == "this") && i + 1 < n && !toks[i + 1].used {
            if let Some(wd) = weekday_of(&toks[i + 1].lower) {
                let d = if w == "next" {
                    next_week_day(today, wd)
                } else {
                    upcoming(today, wd, true)
                };
                lead(toks, 2);
                return Some(d);
            }
            if w == "next" && i + 1 < n {
                let step = match toks[i + 1].lower.as_str() {
                    "week" => Some(7),
                    _ => None,
                };
                if let Some(days) = step {
                    lead(toks, 2);
                    return today.checked_add(days.days()).ok();
                }
            }
        }

        // weekday
        if let Some(wd) = weekday_of(&w) {
            // A bare "sun" or "mon" in a title is rare; accept it, as Fantastical does.
            lead(toks, 1);
            return Some(upcoming(today, wd, false));
        }

        // in N days/weeks/months
        if w == "in"
            && i + 2 < n
            && !toks[i + 1].used
            && !toks[i + 2].used
            && let Ok(k) = toks[i + 1].lower.parse::<i32>()
        {
            let unit = toks[i + 2].lower.as_str();
            let d = match unit {
                "day" | "days" => today.checked_add(k.days()).ok(),
                "week" | "weeks" => today.checked_add((k * 7).days()).ok(),
                "month" | "months" => today.checked_add(k.months()).ok(),
                _ => None,
            };
            if let Some(d) = d {
                lead(toks, 3);
                return Some(d);
            }
        }

        // "jan 5", "january 5th [2027]"
        if let Some(month) = month_of(&w)
            && i + 1 < n
            && !toks[i + 1].used
            && let Some(day) = day_number(&toks[i + 1].lower)
        {
            let (year, used) = year_after(toks, i + 2);
            if let Some(d) = resolve_md(today, month, day, year) {
                lead(toks, 2 + used);
                return Some(d);
            }
        }
        // "5 jan", "5th january [2027]"
        if let Some(day) = day_number(&w)
            && i + 1 < n
            && !toks[i + 1].used
            && let Some(month) = month_of(&toks[i + 1].lower)
        {
            let (year, used) = year_after(toks, i + 2);
            if let Some(d) = resolve_md(today, month, day, year) {
                lead(toks, 2 + used);
                return Some(d);
            }
        }
    }
    None
}

fn year_after(toks: &[Tok], i: usize) -> (Option<i16>, usize) {
    match toks.get(i) {
        Some(t) if !t.used && t.lower.len() == 4 => match t.lower.parse::<i16>() {
            Ok(y) if (2000..=2200).contains(&y) => (Some(y), 1),
            _ => (None, 0),
        },
        _ => (None, 0),
    }
}

/// Month/day with an optional year; without one, the next occurrence on or after `today`.
fn resolve_md(today: Date, month: i8, day: i8, year: Option<i16>) -> Option<Date> {
    if let Some(y) = year {
        return Date::new(y, month, day).ok();
    }
    let this_year = Date::new(today.year(), month, day).ok()?;
    if this_year >= today {
        Some(this_year)
    } else {
        Date::new(today.year() + 1, month, day).ok()
    }
}

/// The next `wd` strictly after `today` (or including it when `inclusive`).
fn upcoming(today: Date, wd: Weekday, inclusive: bool) -> Date {
    let diff = (wd.to_monday_zero_offset() - today.weekday().to_monday_zero_offset()).rem_euclid(7);
    let diff = if diff == 0 && !inclusive { 7 } else { diff };
    today.checked_add((diff as i32).days()).unwrap_or(today)
}

/// `next monday`: that weekday in the calendar week after this one (weeks start Monday).
fn next_week_day(today: Date, wd: Weekday) -> Date {
    let monday = crate::model::week_start(today);
    crate::model::add_days(monday, 7 + wd.to_monday_zero_offset() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::tz::TimeZone;

    /// Wednesday 2026-11-25 10:00 UTC.
    fn now() -> Zoned {
        // No tz database in the Nix sandbox: build the zone from the constant.
        "2026-11-25T10:00:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .to_zoned(TimeZone::UTC)
    }

    fn timed(p: &Parsed) -> (String, String) {
        match p.when {
            Some(When::Timed { start, end }) => (
                start
                    .to_zoned(TimeZone::UTC)
                    .strftime("%Y-%m-%d %H:%M")
                    .to_string(),
                end.to_zoned(TimeZone::UTC)
                    .strftime("%Y-%m-%d %H:%M")
                    .to_string(),
            ),
            ref other => panic!("expected timed, got {other:?}"),
        }
    }

    fn all_day(p: &Parsed) -> (String, String) {
        match p.when {
            Some(When::AllDay { start, end }) => (start.to_string(), end.to_string()),
            ref other => panic!("expected all-day, got {other:?}"),
        }
    }

    #[test]
    fn the_flagship_example() {
        let p = parse("Lunch with Sam tomorrow 1pm at Dishoom", &now());
        assert_eq!(p.title, "Lunch with Sam");
        assert_eq!(p.location.as_deref(), Some("Dishoom"));
        assert_eq!(
            timed(&p),
            ("2026-11-26 13:00".into(), "2026-11-26 14:00".into())
        );
    }

    #[test]
    fn title_only_has_no_when() {
        let p = parse("Buy milk", &now());
        assert_eq!(p.title, "Buy milk");
        assert!(p.when.is_none());
    }

    #[test]
    fn time_only_rolls_to_tomorrow_when_past() {
        let p = parse("Standup 9am", &now());
        assert_eq!(timed(&p).0, "2026-11-26 09:00");
        let p = parse("Call 3pm", &now());
        assert_eq!(timed(&p).0, "2026-11-25 15:00");
    }

    #[test]
    fn date_only_is_all_day() {
        let p = parse("Mum's birthday dec 3", &now());
        assert_eq!(p.title, "Mum's birthday");
        assert_eq!(all_day(&p), ("2026-12-03".into(), "2026-12-04".into()));
        let p = parse("Offsite on friday", &now());
        assert_eq!(p.title, "Offsite");
        assert_eq!(all_day(&p).0, "2026-11-27");
    }

    #[test]
    fn weekdays() {
        // Today is Wednesday.
        assert_eq!(all_day(&parse("x wed", &now())).0, "2026-12-02");
        assert_eq!(all_day(&parse("x this wed", &now())).0, "2026-11-25");
        assert_eq!(all_day(&parse("x thu", &now())).0, "2026-11-26");
        assert_eq!(all_day(&parse("x next mon", &now())).0, "2026-11-30");
        assert_eq!(all_day(&parse("x next friday", &now())).0, "2026-12-04");
    }

    #[test]
    fn explicit_all_day_flag() {
        let p = parse("Conference all day tomorrow", &now());
        assert_eq!(p.title, "Conference");
        assert_eq!(all_day(&p).0, "2026-11-26");
    }

    #[test]
    fn ranges() {
        let p = parse("Workshop tomorrow 1-2pm", &now());
        assert_eq!(
            timed(&p),
            ("2026-11-26 13:00".into(), "2026-11-26 14:00".into())
        );
        let p = parse("Workshop tomorrow 11-1pm", &now());
        assert_eq!(
            timed(&p),
            ("2026-11-26 11:00".into(), "2026-11-26 13:00".into())
        );
        let p = parse("Review tomorrow from 9:30 to 10:45", &now());
        assert_eq!(p.title, "Review");
        assert_eq!(
            timed(&p),
            ("2026-11-26 09:30".into(), "2026-11-26 10:45".into())
        );
        let p = parse("Focus tomorrow 2pm - 4pm", &now());
        assert_eq!(
            timed(&p),
            ("2026-11-26 14:00".into(), "2026-11-26 16:00".into())
        );
    }

    #[test]
    fn durations() {
        let p = parse("Deep work tomorrow 9am for 2h", &now());
        assert_eq!(p.title, "Deep work");
        assert_eq!(
            timed(&p),
            ("2026-11-26 09:00".into(), "2026-11-26 11:00".into())
        );
        let p = parse("Chat tomorrow 9am for 90 minutes", &now());
        assert_eq!(timed(&p).1, "2026-11-26 10:30");
        let p = parse("Chat tomorrow 9am 45min", &now());
        assert_eq!(timed(&p).1, "2026-11-26 09:45");
    }

    #[test]
    fn bare_hours_need_at() {
        let p = parse("Dinner tomorrow at 7", &now());
        assert_eq!(timed(&p).0, "2026-11-26 07:00");
        let p = parse("Beers tomorrow at 5", &now());
        assert_eq!(timed(&p).0, "2026-11-26 17:00");
        // "Room 5" must not become a time.
        let p = parse("Meet in room 5 tomorrow", &now());
        assert_eq!(all_day(&p).0, "2026-11-26");
        assert!(p.title.contains("room 5"));
    }

    #[test]
    fn noon_and_midnight() {
        assert_eq!(
            timed(&parse("Lunch tomorrow noon", &now())).0,
            "2026-11-26 12:00"
        );
        assert_eq!(
            timed(&parse("Fireworks dec 31 midnight", &now())).0,
            "2026-12-31 00:00"
        );
    }

    #[test]
    fn spaced_meridiem() {
        assert_eq!(
            timed(&parse("Lunch tomorrow 1 pm", &now())).0,
            "2026-11-26 13:00"
        );
        assert_eq!(
            timed(&parse("Lunch tomorrow 1:15 pm", &now())).0,
            "2026-11-26 13:15"
        );
    }

    #[test]
    fn twenty_four_hour() {
        assert_eq!(
            timed(&parse("Deploy tomorrow 14:30", &now())).0,
            "2026-11-26 14:30"
        );
    }

    #[test]
    fn relative_times_and_dates() {
        let p = parse("Tea in 2 hours", &now());
        assert_eq!(p.title, "Tea");
        assert_eq!(
            timed(&p),
            ("2026-11-25 12:00".into(), "2026-11-25 13:00".into())
        );
        let p = parse("Follow up in 3 days", &now());
        assert_eq!(all_day(&p).0, "2026-11-28");
        let p = parse("Review in 2 weeks", &now());
        assert_eq!(all_day(&p).0, "2026-12-09");
    }

    #[test]
    fn month_names_and_years() {
        assert_eq!(all_day(&parse("Trip 5 jan", &now())).0, "2027-01-05");
        assert_eq!(all_day(&parse("Trip january 5th", &now())).0, "2027-01-05");
        assert_eq!(all_day(&parse("Trip 5 jan 2028", &now())).0, "2028-01-05");
        assert_eq!(all_day(&parse("Trip 2026-12-01", &now())).0, "2026-12-01");
        // Today's date stays this year.
        assert_eq!(all_day(&parse("x nov 25", &now())).0, "2026-11-25");
    }

    #[test]
    fn locations() {
        let p = parse("Coffee tomorrow 10am @ Blue Bottle Cafe", &now());
        assert_eq!(p.title, "Coffee");
        assert_eq!(p.location.as_deref(), Some("Blue Bottle Cafe"));
        let p = parse("Coffee at Blue Bottle tomorrow 10am", &now());
        assert_eq!(p.location.as_deref(), Some("Blue Bottle"));
        assert_eq!(p.title, "Coffee");
    }

    #[test]
    fn guest_emails_do_not_leave_a_dangling_with() {
        let p = parse("Standup with ann@example.com every weekday 9:30am", &now());
        assert_eq!(p.title, "Standup");
        let p = parse("Lunch with Sam tomorrow 1pm", &now());
        assert_eq!(p.title, "Lunch with Sam");
    }

    #[test]
    fn guests_calendar_and_meet() {
        let p = parse("Sync with ann@example.com tomorrow 3pm /work +meet", &now());
        assert_eq!(p.title, "Sync");
        assert_eq!(p.guests, vec!["ann@example.com".to_string()]);
        assert_eq!(p.calendar_hint.as_deref(), Some("work"));
        assert!(p.meet);
    }

    #[test]
    fn recurrence() {
        let p = parse("Standup every weekday 9:30am", &now());
        assert_eq!(p.title, "Standup");
        assert_eq!(
            p.recurrence.as_ref().map(|r| r.rrule.as_str()),
            Some("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR")
        );
        assert_eq!(timed(&p).0, "2026-11-26 09:30");
        let p = parse("Gym every monday 7am", &now());
        assert_eq!(p.recurrence.unwrap().rrule, "FREQ=WEEKLY;BYDAY=MO");
        let p = parse("Rent monthly", &now());
        assert_eq!(p.recurrence.unwrap().rrule, "FREQ=MONTHLY");
    }

    #[test]
    fn range_crossing_midnight_ends_next_day() {
        let p = parse("Party tomorrow 10pm to 2am", &now());
        assert_eq!(
            timed(&p),
            ("2026-11-26 22:00".into(), "2026-11-27 02:00".into())
        );
    }

    #[test]
    fn weekday_words_in_titles_are_a_known_tradeoff() {
        // "sun" is a weekday; documented behaviour, same as Fantastical.
        let p = parse("Sun cream reminder", &now());
        assert!(p.when.is_some());
    }

    #[test]
    fn garbage_in_does_not_panic() {
        for s in [
            "", " ", "at", "@", "for", "in 2", "1-", "-pm", "at pm", "99:99", "jan 99", "every",
        ] {
            let _ = parse(s, &now());
        }
    }

    #[test]
    fn a_leading_todo_marks_a_task_but_a_bare_task_does_not() {
        let now = now();
        let p = parse("todo Buy milk friday", &now);
        assert!(p.task);
        assert_eq!(p.title, "Buy milk");
        assert!(parse("task: Call Sam", &now).task);
        assert!(!parse("Task force sync friday", &now).task);
        // Alone, it is just a title.
        let p = parse("todo", &now);
        assert!(!p.task);
        assert_eq!(p.title, "todo");
    }
}
