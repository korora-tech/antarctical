//! Subcommands that talk to the running Antarctical over its socket instead of starting the
//! app: `events` prints the agenda, `vdir` mirrors the calendars into a directory (see
//! `vdir.rs`), `show` brings the window forward and `refresh` syncs.
//!
//! `events --watch` is what desktop-shell integrations read (the Noctalia plugin runs it as
//! a stream). It never exits on its own: it prints the agenda whenever it changes, says when
//! Antarctical is not running, and keeps looking for it. One JSON object per line:
//!
//! - `{"type":"begin"}`, one `{"type":"event",…}` per event, `{"type":"end","events":N}`: a
//!   complete agenda. Readers apply it at `end`; a reader that sees `begin` twice discards
//!   the first.
//! - `{"type":"offline"}`: Antarctical is not running.
//! - `{"type":"ping"}`: nothing changed for a while. It tells a reader the stream is alive
//!   and lets a watcher whose reader went away notice and exit.
//!
//! Events are split over lines because shells cap the length of a stream line (Noctalia at
//! 64 KiB), which a single document with a week of meetings can exceed.

use std::convert::Infallible;
use std::io::{self, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};
use serde::Serialize;
use serde_json::{Value, json};

use crate::dankcal::{self, Client, DECLINED_SUFFIX};
use crate::model::add_days;
use crate::vdir;

pub const USAGE: &str = "       antarctical events [--days N] [--watch]
       antarctical vdir DIR [--back MONTHS] [--ahead MONTHS]
       antarctical show [--date YYYY-MM-DD]
       antarctical refresh

  events     print the running Antarctical's agenda as JSON: today and the days after it
             (--days, default 7). With --watch, keep printing it as it changes, one JSON
             object per line, for desktop-shell widgets
  vdir       keep DIR mirroring the calendars shown in the sidebar, one .ics file per
             event, for anything that reads a vdir (Noctalia's calendar, khal). Covers
             last month to six months ahead unless --back and --ahead say otherwise.
             Runs until stopped
  show       bring the running Antarctical's window forward, optionally on a date
  refresh    ask the running Antarctical to sync now

These talk to the running Antarctical and exit with status 3 when there is none
(events --watch and vdir wait for it instead).";

const NOT_RUNNING: u8 = 3;
const DEFAULT_DAYS: i32 = 7;
/// A shell asks for a few days; the socket refuses ranges over a year.
const MAX_DAYS: i32 = 92;
/// How long `--watch` waits before looking for Antarctical again.
const RETRY: Duration = Duration::from_secs(3);
/// `--watch` re-reads the agenda this often even without a push (the window moves at
/// midnight), and prints a ping when it has been silent this long.
const HEARTBEAT: Duration = Duration::from_secs(30);
/// Pushes arriving this close together are answered with one read of the agenda.
const SETTLE: Duration = Duration::from_millis(150);
/// Keeps one event well inside a 64 KiB stream line.
const MAX_TEXT: usize = 300;
const MAX_URL: usize = 2048;

/// Runs `args` as a subcommand and returns its exit status. `None`: `args` is not a
/// subcommand, so the app itself starts.
pub fn run(args: &[String]) -> Option<u8> {
    let (name, rest) = args.split_first()?;
    let command = match name.as_str() {
        "events" => parse_events(rest),
        "vdir" => parse_vdir(rest),
        "show" => parse_show(rest),
        "refresh" => rest
            .is_empty()
            .then_some(Subcommand::Refresh)
            .ok_or_else(|| format!("unexpected argument {}", rest[0])),
        _ => return None,
    };
    Some(match command {
        Ok(command) => execute(command),
        Err(error) => {
            eprintln!("antarctical {name}: {error}\nusage:\n{USAGE}");
            2
        }
    })
}

#[derive(Debug, PartialEq)]
enum Subcommand {
    Events { days: i32, watch: bool },
    Vdir { dir: PathBuf, back: i8, ahead: i8 },
    Show { date: Option<Date> },
    Refresh,
}

fn parse_events(args: &[String]) -> Result<Subcommand, String> {
    let mut days = DEFAULT_DAYS;
    let mut watch = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--watch" => watch = true,
            "--days" => {
                let value = args.next().ok_or("--days needs a number")?;
                days = value
                    .parse()
                    .ok()
                    .filter(|d| (1..=MAX_DAYS).contains(d))
                    .ok_or(format!("--days must be between 1 and {MAX_DAYS}"))?;
            }
            other => return Err(format!("unexpected argument {other}")),
        }
    }
    Ok(Subcommand::Events { days, watch })
}

fn parse_vdir(args: &[String]) -> Result<Subcommand, String> {
    let (mut dir, mut back, mut ahead) = (None, vdir::DEFAULT_BACK, vdir::DEFAULT_AHEAD);
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let months = match arg.as_str() {
            "--back" => &mut back,
            "--ahead" => &mut ahead,
            flag if flag.starts_with('-') => return Err(format!("unexpected argument {flag}")),
            path if dir.is_none() => {
                dir = Some(PathBuf::from(path));
                continue;
            }
            extra => return Err(format!("unexpected argument {extra}")),
        };
        *months = args
            .next()
            .and_then(|value| value.parse().ok())
            .filter(|m| (0..=vdir::MAX_MONTHS).contains(m))
            .ok_or(format!("{arg} needs a number of months"))?;
    }
    if back + ahead > vdir::MAX_MONTHS {
        return Err(format!(
            "--back and --ahead may cover {} months together",
            vdir::MAX_MONTHS
        ));
    }
    let dir = dir.ok_or("which directory?")?;
    Ok(Subcommand::Vdir { dir, back, ahead })
}

fn parse_show(args: &[String]) -> Result<Subcommand, String> {
    match args {
        [] => Ok(Subcommand::Show { date: None }),
        [flag, value] if flag == "--date" => value
            .parse()
            .map(|date| Subcommand::Show { date: Some(date) })
            .map_err(|_| format!("--date must be YYYY-MM-DD, not {value}")),
        [flag] if flag == "--date" => Err("--date needs a date".into()),
        [other, ..] => Err(format!("unexpected argument {other}")),
    }
}

fn execute(command: Subcommand) -> u8 {
    match command {
        Subcommand::Events { days, watch: true } => watch(days),
        Subcommand::Events { days, watch: false } => once(|client| {
            let events = agenda(client, days, &Zoned::now())?;
            println!("{}", json!({ "events": events }));
            Ok(())
        }),
        Subcommand::Vdir { dir, back, ahead } => vdir::run(&dir, back, ahead),
        Subcommand::Show { date } => once(|client| {
            let params = match date {
                Some(date) => json!({ "date": date.to_string() }),
                None => Value::Null,
            };
            client.request("antarctical.show", params).map(drop)
        }),
        Subcommand::Refresh => {
            once(|client| client.request("antarctical.refresh", Value::Null).map(drop))
        }
    }
}

/// Runs one exchange with the running Antarctical.
fn once(exchange: impl FnOnce(&mut Client) -> io::Result<()>) -> u8 {
    let Some(mut client) = connect() else {
        eprintln!("antarctical: not running");
        return NOT_RUNNING;
    };
    match exchange(&mut client) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("antarctical: the running instance did not answer: {error}");
            1
        }
    }
}

fn connect() -> Option<Client> {
    Client::connect(&dankcal::running_socket()?).ok()
}

// ----- the agenda -------------------------------------------------------------------------

/// One event as shell integrations read it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgendaEvent {
    pub id: String,
    pub title: String,
    /// The calendar's name.
    pub calendar: String,
    /// The calendar's colour, `#rrggbb`.
    pub color: String,
    /// Milliseconds since the Unix epoch. An all-day event starts and ends at local midnight
    /// (the end is exclusive).
    pub start_ms: i64,
    pub end_ms: i64,
    pub all_day: bool,
    pub location: String,
    /// The event's page in Google Calendar. Only ever `http(s)`, or empty.
    pub url: String,
    /// The video call. Only ever `http(s)`, or empty.
    pub meeting_url: String,
    pub tentative: bool,
    /// Listed only while Antarctical shows declined events.
    pub declined: bool,
}

/// Reads the events of today and the `days - 1` days after it. Asking also has Antarctical
/// keep those months synced while this connection is open.
fn agenda(client: &mut Client, days: i32, now: &Zoned) -> io::Result<Vec<AgendaEvent>> {
    let tz = now.time_zone();
    let today = now.date();
    let instant = |date: Date| date.to_zoned(tz.clone()).map(|z| z.timestamp());
    let (Ok(from), Ok(to)) = (instant(today), instant(add_days(today, days))) else {
        return Err(io::Error::other("the agenda's days are out of range"));
    };
    let calendars = client.request("calendars.list", Value::Null)?;
    let events = client.request(
        "events.list",
        json!({ "from": from.to_string(), "to": to.to_string() }),
    )?;
    Ok(agenda_events(&calendars, &events["events"], tz))
}

/// Turns the socket's calendars and events into the agenda: events of calendars hidden in
/// the sidebar are left out, and all-day events are placed at local midnight in `tz`.
fn agenda_events(calendars: &Value, events: &Value, tz: &TimeZone) -> Vec<AgendaEvent> {
    let calendar = |id: &str| {
        calendars
            .as_array()
            .into_iter()
            .flatten()
            .find(|c| c["id"] == id)
    };
    let mut out: Vec<AgendaEvent> = events
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|e| {
            let listed_under = e["calendarId"].as_str()?;
            let declined = listed_under.ends_with(DECLINED_SUFFIX);
            let cal = calendar(listed_under.trim_end_matches(DECLINED_SUFFIX))?;
            if cal["hidden"] == true {
                return None;
            }
            let all_day = e["allDay"] == true;
            let instant = |key: &str| {
                let text = e[key].as_str()?;
                if all_day {
                    let date: Date = text.get(..10)?.parse().ok()?;
                    Some(date.to_zoned(tz.clone()).ok()?.timestamp())
                } else {
                    text.parse::<Timestamp>().ok()
                }
            };
            let text = |key: &str, max: usize| clip(e[key].as_str().unwrap_or_default(), max);
            Some(AgendaEvent {
                id: text("id", MAX_URL),
                title: text("summary", MAX_TEXT),
                calendar: clip(cal["name"].as_str().unwrap_or_default(), MAX_TEXT),
                color: clip(cal["color"].as_str().unwrap_or_default(), 9),
                start_ms: instant("start")?.as_millisecond(),
                end_ms: instant("end")?.as_millisecond(),
                all_day,
                location: text("location", MAX_TEXT),
                url: web_url(e["url"].as_str()),
                meeting_url: web_url(e["meetingUrl"].as_str()),
                tentative: e["status"] == "tentative",
                declined,
            })
        })
        .collect();
    out.sort_by(|a, b| (a.start_ms, &a.id).cmp(&(b.start_ms, &b.id)));
    out
}

/// `text` cut to at most `max` bytes on a character boundary.
fn clip(text: &str, max: usize) -> String {
    let mut end = text.len().min(max);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

/// Readers hand these to `xdg-open`, so anything that is not a web address is dropped.
fn web_url(url: Option<&str>) -> String {
    url.filter(|u| u.len() <= MAX_URL && (u.starts_with("https://") || u.starts_with("http://")))
        .unwrap_or_default()
        .to_string()
}

// ----- watching ---------------------------------------------------------------------------

/// A line of `events --watch`.
#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Line<'a> {
    Begin,
    Event(&'a AgendaEvent),
    End { events: usize },
    Offline,
    Ping,
}

/// What `--watch` last told its reader.
#[derive(Debug, PartialEq)]
enum Told {
    Offline,
    Agenda(Vec<AgendaEvent>),
}

/// The watch stream. Remembers what was written so unchanged agendas are not repeated.
struct Stream<W: Write> {
    out: W,
    told: Option<Told>,
    last_write: Instant,
}

impl<W: Write> Stream<W> {
    fn new(out: W) -> Self {
        Self {
            out,
            told: None,
            last_write: Instant::now(),
        }
    }

    fn write(&mut self, lines: &[Line]) -> io::Result<()> {
        for line in lines {
            serde_json::to_writer(&mut self.out, line).map_err(io::Error::other)?;
            self.out.write_all(b"\n")?;
        }
        self.last_write = Instant::now();
        self.out.flush()
    }

    /// Writes `next` if it differs from what the reader has, or a ping once the stream has
    /// been silent for [`HEARTBEAT`].
    fn tell(&mut self, next: Told) -> io::Result<()> {
        if self.told.as_ref() == Some(&next) {
            return if self.last_write.elapsed() >= HEARTBEAT {
                self.write(&[Line::Ping])
            } else {
                Ok(())
            };
        }
        match &next {
            Told::Offline => self.write(&[Line::Offline])?,
            Told::Agenda(events) => {
                let mut lines = vec![Line::Begin];
                lines.extend(events.iter().map(Line::Event));
                lines.push(Line::End {
                    events: events.len(),
                });
                self.write(&lines)?;
            }
        }
        self.told = Some(next);
        Ok(())
    }
}

/// Follows the running Antarctical until the reader goes away (a failed write).
fn watch(days: i32) -> u8 {
    let mut stream = Stream::new(io::stdout().lock());
    loop {
        let gone = match follow(&mut stream, days) {
            Err(gone) => gone,
            Ok(never) => match never {},
        };
        if gone == Gone::Reader || stream.tell(Told::Offline).is_err() {
            return 0;
        }
        std::thread::sleep(RETRY);
    }
}

#[derive(Debug, PartialEq)]
enum Gone {
    /// Antarctical is not running, or its connection dropped.
    App,
    /// Nobody reads the stream any more.
    Reader,
}

/// Prints the agenda now and after every change, on one connection that hears about changes
/// and another that reads them. Returns only once Antarctical or the reader is gone.
fn follow<W: Write>(stream: &mut Stream<W>, days: i32) -> Result<Infallible, Gone> {
    let app = |_| Gone::App;
    let path = dankcal::running_socket().ok_or(Gone::App)?;
    let mut pushes = Client::connect(&path).map_err(app)?;
    pushes
        .request("subscribe", json!({ "topics": ["calendars", "events"] }))
        .map_err(app)?;
    let mut requests = Client::connect(&path).map_err(app)?;
    loop {
        let events = agenda(&mut requests, days, &Zoned::now()).map_err(app)?;
        stream
            .tell(Told::Agenda(events))
            .map_err(|_| Gone::Reader)?;
        if pushes.next_push(HEARTBEAT).map_err(app)?.is_some() {
            // A sync announces calendars and events separately, and month by month.
            while pushes.next_push(SETTLE).map_err(app)?.is_some() {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn args(text: &str) -> Vec<String> {
        text.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn subcommands_parse_and_everything_else_starts_the_app() {
        assert_eq!(
            parse_events(&args("--watch --days 14")),
            Ok(Subcommand::Events {
                days: 14,
                watch: true
            })
        );
        assert_eq!(
            parse_events(&[]),
            Ok(Subcommand::Events {
                days: DEFAULT_DAYS,
                watch: false
            })
        );
        assert!(parse_events(&args("--days 0")).is_err());
        assert!(parse_events(&args("--days 93")).is_err());
        assert!(parse_events(&args("--days")).is_err());
        assert!(parse_events(&args("--nope")).is_err());
        assert_eq!(
            parse_show(&args("--date 2026-10-21")),
            Ok(Subcommand::Show {
                date: Some(date(2026, 10, 21))
            })
        );
        assert_eq!(parse_show(&[]), Ok(Subcommand::Show { date: None }));
        assert_eq!(
            parse_vdir(&args("/tmp/cal --ahead 3")),
            Ok(Subcommand::Vdir {
                dir: "/tmp/cal".into(),
                back: vdir::DEFAULT_BACK,
                ahead: 3
            })
        );
        assert!(parse_vdir(&[]).is_err());
        assert!(parse_vdir(&args("a b")).is_err());
        assert!(parse_vdir(&args("a --back 6 --ahead 6")).is_err());
        assert!(parse_vdir(&args("a --ahead")).is_err());
        assert!(parse_show(&args("--date tomorrow")).is_err());
        assert!(parse_show(&args("--date")).is_err());

        assert_eq!(run(&args("--hidden")), None);
        assert_eq!(run(&[]), None);
        assert_eq!(run(&args("events --nope")), Some(2));
        assert_eq!(run(&args("refresh now")), Some(2));
    }

    fn calendars() -> Value {
        json!([
            {"id": "me@x/work", "name": "Work", "color": "#33b679", "hidden": false},
            {"id": "me@x/work#declined", "name": "Work (declined)", "color": "#9e9e9e", "hidden": false},
            {"id": "me@x/side", "name": "Side", "color": "#f4511e", "hidden": true},
        ])
    }

    fn wire(id: &str, calendar: &str, start: &str, end: &str) -> Value {
        json!({
            "id": id,
            "calendarId": calendar,
            "summary": id,
            "location": "Room 4",
            "url": "https://calendar.google.com/x",
            "meetingUrl": "https://meet.google.com/abc",
            "start": start,
            "end": end,
            "allDay": false,
            "status": "confirmed",
        })
    }

    #[test]
    fn agenda_carries_what_a_widget_shows() {
        let events = json!([wire(
            "standup",
            "me@x/work",
            "2026-10-21T09:30:00Z",
            "2026-10-21T09:45:00Z"
        )]);
        let agenda = agenda_events(&calendars(), &events, &TimeZone::UTC);
        assert_eq!(
            agenda,
            vec![AgendaEvent {
                id: "standup".into(),
                title: "standup".into(),
                calendar: "Work".into(),
                color: "#33b679".into(),
                start_ms: 1_792_575_000_000,
                end_ms: 1_792_575_900_000,
                all_day: false,
                location: "Room 4".into(),
                url: "https://calendar.google.com/x".into(),
                meeting_url: "https://meet.google.com/abc".into(),
                tentative: false,
                declined: false,
            }]
        );
        let line = serde_json::to_value(Line::Event(&agenda[0])).unwrap();
        assert_eq!(line["type"], "event");
        assert_eq!(line["startMs"], 1_792_575_000_000_i64);
        assert_eq!(line["meetingUrl"], "https://meet.google.com/abc");
        assert_eq!(line["allDay"], false);
    }

    #[test]
    fn all_day_events_sit_at_local_midnight() {
        let mut holiday = wire(
            "holiday",
            "me@x/work",
            "2026-10-21T00:00:00Z",
            "2026-10-22T00:00:00Z",
        );
        holiday["allDay"] = json!(true);
        let events = json!([holiday]);
        let utc = agenda_events(&calendars(), &events, &TimeZone::UTC);
        let tokyo = agenda_events(&calendars(), &events, &TimeZone::fixed(jiff::tz::offset(9)));
        assert!(utc[0].all_day);
        let nine_hours = 9 * 3600 * 1000;
        assert_eq!(tokyo[0].start_ms, utc[0].start_ms - nine_hours);
        assert_eq!(tokyo[0].end_ms, utc[0].end_ms - nine_hours);
        assert_eq!(utc[0].end_ms - utc[0].start_ms, 24 * 3600 * 1000);
    }

    #[test]
    fn hidden_calendars_are_left_out_and_declined_events_are_marked() {
        let mut tentative = wire(
            "maybe",
            "me@x/work",
            "2026-10-21T08:00:00Z",
            "2026-10-21T09:00:00Z",
        );
        tentative["status"] = json!("tentative");
        let events = json!([
            wire(
                "hidden",
                "me@x/side",
                "2026-10-21T10:00:00Z",
                "2026-10-21T11:00:00Z"
            ),
            wire(
                "nope",
                "me@x/work#declined",
                "2026-10-21T12:00:00Z",
                "2026-10-21T13:00:00Z"
            ),
            wire(
                "orphan",
                "me@x/gone",
                "2026-10-21T12:00:00Z",
                "2026-10-21T13:00:00Z"
            ),
            tentative,
        ]);
        let agenda = agenda_events(&calendars(), &events, &TimeZone::UTC);
        let ids: Vec<&str> = agenda.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["maybe", "nope"], "sorted by start");
        assert!(agenda[0].tentative && !agenda[0].declined);
        assert!(agenda[1].declined);
        assert_eq!(
            agenda[1].calendar, "Work",
            "its own calendar, not the grey one"
        );
        assert_eq!(agenda[1].color, "#33b679");
    }

    #[test]
    fn only_web_addresses_reach_the_reader_and_long_text_is_cut() {
        let mut odd = wire(
            "odd",
            "me@x/work",
            "2026-10-21T08:00:00Z",
            "2026-10-21T09:00:00Z",
        );
        odd["url"] = json!("file:///etc/passwd");
        odd["meetingUrl"] = json!("javascript:alert(1)");
        odd["summary"] = json!("é".repeat(400));
        let agenda = agenda_events(&calendars(), &json!([odd]), &TimeZone::UTC);
        assert_eq!(agenda[0].url, "");
        assert_eq!(agenda[0].meeting_url, "");
        assert_eq!(agenda[0].title, "é".repeat(MAX_TEXT / 2));
        assert_eq!(clip("añb", 2), "a", "never splits a character");
    }

    fn lines(out: &[u8]) -> Vec<Value> {
        std::str::from_utf8(out)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    #[test]
    fn the_stream_frames_agendas_and_stays_quiet_when_nothing_changed() {
        let events = agenda_events(
            &calendars(),
            &json!([wire(
                "standup",
                "me@x/work",
                "2026-10-21T09:30:00Z",
                "2026-10-21T09:45:00Z"
            )]),
            &TimeZone::UTC,
        );
        let mut stream = Stream::new(Vec::new());
        stream.tell(Told::Agenda(events.clone())).unwrap();
        stream.tell(Told::Agenda(events.clone())).unwrap();
        stream.tell(Told::Offline).unwrap();
        stream.tell(Told::Offline).unwrap();
        stream.tell(Told::Agenda(vec![])).unwrap();
        let written = lines(&stream.out);
        let types: Vec<&str> = written
            .iter()
            .map(|l| l["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            types,
            vec!["begin", "event", "end", "offline", "begin", "end"]
        );
        assert_eq!(written[1]["id"], "standup");
        assert_eq!(written[2]["events"], 1);
        assert_eq!(written[5]["events"], 0);
    }

    #[test]
    fn a_silent_stream_pings() {
        let mut stream = Stream::new(Vec::new());
        stream.tell(Told::Offline).unwrap();
        stream.last_write = Instant::now() - HEARTBEAT;
        stream.tell(Told::Offline).unwrap();
        let written = lines(&stream.out);
        assert_eq!(
            written,
            vec![json!({"type": "offline"}), json!({"type": "ping"})]
        );
    }

    #[test]
    fn a_reader_that_went_away_ends_the_watch() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut stream = Stream::new(Closed);
        assert!(stream.tell(Told::Offline).is_err());
        assert_eq!(stream.told, None, "nothing was told");
    }
}
