//! `antarctical vdir <dir>`: mirrors the running Antarctical's calendars into a vdir, the
//! directory layout vdirsyncer and khal use, so anything that reads one shows Antarctical's
//! events. Noctalia's own calendar does, with a `type = "vdir"` account.
//!
//! ```text
//! <dir>/<collection>/.antarctical    marks the collection as written by this command
//! <dir>/<collection>/displayname     the calendar's name
//! <dir>/<collection>/color           its colour, #rrggbb
//! <dir>/<collection>/<event>.ics     one event (Google already expands repeating ones)
//! ```
//!
//! The mirror follows the sidebar: hidden calendars and declined events are left out. It
//! covers a window of months around today, is updated whenever Antarctical's events change,
//! and is left as it is while Antarctical is not running. Only collections carrying the
//! marker are ever changed or removed, so the directory can be shared with other tools.
//!
//! Events carry no alarms: Antarctical sends the reminders itself.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use base64::Engine;
use jiff::civil::Date;
use jiff::{ToSpan, Zoned};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::dankcal::{self, Client, DECLINED_SUFFIX};
use crate::model::first_of_month;

pub const DEFAULT_BACK: i8 = 1;
pub const DEFAULT_AHEAD: i8 = 6;
/// Together with the current month this stays inside the year the socket serves.
pub const MAX_MONTHS: i8 = 10;

const MARKER: &str = ".antarctical";
const RETRY: Duration = Duration::from_secs(3);
/// The window moves with the calendar, so the mirror is rebuilt this often without a push.
const HEARTBEAT: Duration = Duration::from_secs(300);
const SETTLE: Duration = Duration::from_millis(500);

/// What one collection directory should hold.
#[derive(Debug, Default, PartialEq)]
struct Collection {
    name: String,
    color: String,
    /// File name → contents.
    events: BTreeMap<String, String>,
}

/// Keeps `dir` mirroring the running Antarctical. Returns only if `dir` cannot be written.
pub fn run(dir: &Path, back: i8, ahead: i8) -> u8 {
    let mut written = None;
    loop {
        if let Err(error) = follow(dir, back, ahead, &mut written) {
            eprintln!("antarctical: cannot write {}: {error}", dir.display());
            return 1;
        }
        std::thread::sleep(RETRY);
    }
}

/// Mirrors until Antarctical goes away (`Ok`) or the directory cannot be written (`Err`).
fn follow(
    dir: &Path,
    back: i8,
    ahead: i8,
    written: &mut Option<BTreeMap<String, Collection>>,
) -> io::Result<()> {
    let Some(path) = dankcal::running_socket() else {
        return Ok(());
    };
    let (Ok(mut pushes), Ok(mut requests)) = (Client::connect(&path), Client::connect(&path))
    else {
        return Ok(());
    };
    if pushes
        .request("subscribe", json!({ "topics": ["calendars", "events"] }))
        .is_err()
    {
        return Ok(());
    }
    loop {
        let Ok(collections) = read(&mut requests, back, ahead, &Zoned::now()) else {
            return Ok(());
        };
        // No calendars: Antarctical is still starting (or signed out mid-sync). Wait for
        // the calendars rather than empty the mirror.
        if let Some(collections) = collections
            && written.as_ref() != Some(&collections)
        {
            sync(dir, &collections)?;
            *written = Some(collections);
        }
        match pushes.next_push(HEARTBEAT) {
            Err(_) => return Ok(()),
            Ok(None) => {}
            // A sync announces calendars and events separately, and month by month.
            Ok(Some(_)) => loop {
                match pushes.next_push(SETTLE) {
                    Err(_) => return Ok(()),
                    Ok(None) => break,
                    Ok(Some(_)) => {}
                }
            },
        }
    }
}

/// Reads the calendars and the events of the window. `None`: no calendars are known yet.
fn read(
    client: &mut Client,
    back: i8,
    ahead: i8,
    now: &Zoned,
) -> io::Result<Option<BTreeMap<String, Collection>>> {
    let this_month = first_of_month(now.date());
    let edge = |months: i8| {
        this_month
            .checked_add(i32::from(months).months())
            .ok()
            .and_then(|date| date.to_zoned(now.time_zone().clone()).ok())
            .map(|zoned| zoned.timestamp().to_string())
            .ok_or_else(|| io::Error::other("the window is out of range"))
    };
    let calendars = client.request("calendars.list", Value::Null)?;
    if calendars.as_array().is_none_or(Vec::is_empty) {
        return Ok(None);
    }
    let events = client.request(
        "events.list",
        json!({ "from": edge(-back)?, "to": edge(ahead + 1)? }),
    )?;
    Ok(Some(collections(&calendars, &events["events"])))
}

/// Turns the socket's calendars and events into collections, keyed by directory name.
fn collections(calendars: &Value, events: &Value) -> BTreeMap<String, Collection> {
    let mut out = BTreeMap::new();
    for cal in calendars.as_array().into_iter().flatten() {
        let Some(id) = cal["id"].as_str() else {
            continue;
        };
        if cal["hidden"] == true || id.ends_with(DECLINED_SUFFIX) {
            continue;
        }
        let events = events
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| e["calendarId"] == id)
            .filter_map(|e| Some((format!("{}.ics", name_for(e["id"].as_str()?)), ics(e)?)))
            .collect();
        out.insert(
            name_for(id),
            Collection {
                name: cal["name"].as_str().unwrap_or_default().to_string(),
                color: cal["color"].as_str().unwrap_or_default().to_string(),
                events,
            },
        );
    }
    out
}

/// A file-system-safe name for an id: ids hold `/`, `@` and are longer than a file name
/// may be.
fn name_for(id: &str) -> String {
    let digest = Sha256::digest(id.as_bytes());
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&digest[..15])
}

/// One event as an iCalendar object. `None` when its times cannot be read.
fn ics(e: &Value) -> Option<String> {
    let text = |key: &str| e[key].as_str().unwrap_or_default();
    let all_day = e["allDay"] == true;
    let when = |name: &str, key: &str| {
        let value = e[key].as_str()?;
        Some(if all_day {
            let date: Date = value.get(..10)?.parse().ok()?;
            format!("{name};VALUE=DATE:{}", date.strftime("%Y%m%d"))
        } else {
            let at: jiff::Timestamp = value.parse().ok()?;
            format!("{name}:{}", at.strftime("%Y%m%dT%H%M%SZ"))
        })
    };
    let start = when("DTSTART", "start")?;
    let stamp = match start.split_once(':')?.1 {
        date if all_day => format!("{date}T000000Z"),
        instant => instant.to_string(),
    };
    // Readers open the first link they find; the video call is the one worth opening.
    let url = [text("meetingUrl"), text("url")]
        .into_iter()
        .find(|u| u.starts_with("https://") || u.starts_with("http://"));
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//Korora//Antarctical//EN".to_string(),
        "BEGIN:VEVENT".to_string(),
        format!("UID:{}", escape(text("id"))),
        // Required, and constant so an unchanged event is an unchanged file.
        format!("DTSTAMP:{stamp}"),
        start,
        when("DTEND", "end")?,
        format!("SUMMARY:{}", escape(text("summary"))),
    ];
    for (name, value) in [
        ("LOCATION", text("location")),
        ("DESCRIPTION", text("description")),
    ] {
        if !value.is_empty() {
            lines.push(format!("{name}:{}", escape(value)));
        }
    }
    if let Some(url) = url {
        lines.push(format!("URL:{url}"));
    }
    if e["status"] == "tentative" {
        lines.push("STATUS:TENTATIVE".to_string());
    }
    lines.push("END:VEVENT".to_string());
    lines.push("END:VCALENDAR".to_string());
    Some(lines.iter().map(|line| fold(line)).collect())
}

/// Escapes a text value (RFC 5545 §3.3.11).
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// Ends a content line with CRLF, folding it so no line is over 75 bytes (RFC 5545 §3.1).
fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 2);
    let mut width = 0;
    for c in line.chars() {
        if width + c.len_utf8() > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += c.len_utf8();
    }
    out.push_str("\r\n");
    out
}

// ----- writing ----------------------------------------------------------------------------

/// Makes `dir` hold exactly `wanted`, touching only collections this command wrote. A file
/// is rewritten only when its contents changed, so readers watching the directory are not
/// woken for nothing.
fn sync(dir: &Path, wanted: &BTreeMap<String, Collection>) -> io::Result<()> {
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    for entry in std::fs::read_dir(dir)?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !wanted.contains_key(&name) && entry.path().join(MARKER).is_file() {
            std::fs::remove_dir_all(entry.path())?;
        }
    }
    for (name, collection) in wanted {
        let path = dir.join(name);
        if path.exists() && !path.join(MARKER).is_file() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("{} was not written by antarctical", path.display()),
            ));
        }
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&path)?;
        put(&path.join(MARKER), "")?;
        put(&path.join("displayname"), &collection.name)?;
        put(&path.join("color"), &collection.color)?;
        let stale: BTreeSet<PathBuf> = std::fs::read_dir(&path)?
            .flatten()
            .map(|entry| entry.path())
            .filter(|file| file.extension().is_some_and(|e| e == "ics"))
            .filter(|file| {
                file.file_name()
                    .and_then(|n| n.to_str())
                    .is_none_or(|n| !collection.events.contains_key(n))
            })
            .collect();
        for file in stale {
            std::fs::remove_file(file)?;
        }
        for (file, contents) in &collection.events {
            put(&path.join(file), contents)?;
        }
    }
    Ok(())
}

/// Writes `contents` to `path` (0600) unless it already holds them. The file appears whole:
/// it is written beside `path` and renamed over it.
fn put(path: &Path, contents: &str) -> io::Result<()> {
    if std::fs::read(path).is_ok_and(|existing| existing == contents.as_bytes()) {
        return Ok(());
    }
    let mut partial = path.as_os_str().to_owned();
    partial.push(".part");
    let partial = PathBuf::from(partial);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&partial)?;
    file.write_all(contents.as_bytes())?;
    drop(file);
    std::fs::rename(&partial, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calendars() -> Value {
        json!([
            {"id": "me@x/work", "name": "Work", "color": "#33b679", "hidden": false},
            {"id": "me@x/work#declined", "name": "Work (declined)", "color": "#9e9e9e", "hidden": false},
            {"id": "me@x/side", "name": "Side", "color": "#f4511e", "hidden": true},
        ])
    }

    fn event(id: &str, calendar: &str) -> Value {
        json!({
            "id": id,
            "calendarId": calendar,
            "summary": "Design review; part 2, final",
            "description": "Agenda:\n1. Demo",
            "location": "Room 4",
            "url": "https://calendar.google.com/x",
            "meetingUrl": "https://meet.google.com/abc",
            "start": "2026-10-21T09:30:00Z",
            "end": "2026-10-21T09:45:00Z",
            "allDay": false,
            "status": "tentative",
        })
    }

    #[test]
    fn a_timed_event_becomes_an_icalendar_object() {
        let text = ics(&event("work/standup", "me@x/work")).unwrap();
        assert_eq!(
            text.split("\r\n").collect::<Vec<_>>(),
            vec![
                "BEGIN:VCALENDAR",
                "VERSION:2.0",
                "PRODID:-//Korora//Antarctical//EN",
                "BEGIN:VEVENT",
                "UID:work/standup",
                "DTSTAMP:20261021T093000Z",
                "DTSTART:20261021T093000Z",
                "DTEND:20261021T094500Z",
                "SUMMARY:Design review\\; part 2\\, final",
                "LOCATION:Room 4",
                "DESCRIPTION:Agenda:\\n1. Demo",
                "URL:https://meet.google.com/abc",
                "STATUS:TENTATIVE",
                "END:VEVENT",
                "END:VCALENDAR",
                "",
            ]
        );
    }

    #[test]
    fn an_all_day_event_uses_dates_and_falls_back_to_its_page() {
        let mut e = event("work/holiday", "me@x/work");
        e["allDay"] = json!(true);
        e["start"] = json!("2026-10-21T00:00:00Z");
        e["end"] = json!("2026-10-22T00:00:00Z");
        e["meetingUrl"] = json!("");
        e["status"] = json!("confirmed");
        let text = ics(&e).unwrap();
        assert!(text.contains("DTSTART;VALUE=DATE:20261021\r\n"));
        assert!(text.contains("DTEND;VALUE=DATE:20261022\r\n"));
        assert!(text.contains("DTSTAMP:20261021T000000Z\r\n"));
        assert!(text.contains("URL:https://calendar.google.com/x\r\n"));
        assert!(!text.contains("STATUS"));

        e["start"] = json!("soon");
        assert_eq!(ics(&e), None);
    }

    #[test]
    fn long_lines_fold_on_character_boundaries() {
        let folded = fold(&format!("SUMMARY:{}", "é".repeat(60)));
        for line in folded.trim_end().split("\r\n") {
            assert!(line.len() <= 75, "{} bytes", line.len());
        }
        let unfolded = folded.trim_end().replace("\r\n ", "");
        assert_eq!(unfolded, format!("SUMMARY:{}", "é".repeat(60)));
        assert_eq!(fold("UID:a"), "UID:a\r\n");
    }

    #[test]
    fn the_mirror_follows_the_sidebar() {
        let events = json!([
            event("work/standup", "me@x/work"),
            event("work/nope", "me@x/work#declined"),
            event("side/x", "me@x/side"),
        ]);
        let all = collections(&calendars(), &events);
        assert_eq!(all.len(), 1, "hidden and declined calendars are left out");
        let work = &all[&name_for("me@x/work")];
        assert_eq!(
            (work.name.as_str(), work.color.as_str()),
            ("Work", "#33b679")
        );
        assert_eq!(work.events.len(), 1);
        let file = work.events.keys().next().unwrap();
        assert!(file.ends_with(".ics") && !file.contains('/'));
    }

    #[test]
    fn syncing_writes_updates_and_prunes_only_its_own_collections() {
        let dir = std::env::temp_dir().join(format!("actl-vdir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("khal")).unwrap();
        std::fs::write(dir.join("khal/mine.ics"), "theirs").unwrap();

        let events = json!([
            event("work/standup", "me@x/work"),
            event("work/retro", "me@x/work")
        ]);
        let first = collections(&calendars(), &events);
        sync(&dir, &first).unwrap();
        let work = dir.join(name_for("me@x/work"));
        let count = |path: &Path| {
            std::fs::read_dir(path)
                .unwrap()
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "ics"))
                .count()
        };
        assert_eq!(count(&work), 2);
        assert_eq!(
            std::fs::read_to_string(work.join("displayname")).unwrap(),
            "Work"
        );
        assert_eq!(
            std::fs::read_to_string(work.join("color")).unwrap(),
            "#33b679"
        );

        // An unchanged event is not rewritten; a removed one goes.
        let standup = work.join(format!("{}.ics", name_for("work/standup")));
        let before = std::fs::metadata(&standup).unwrap().modified().unwrap();
        let second = collections(&calendars(), &json!([event("work/standup", "me@x/work")]));
        sync(&dir, &second).unwrap();
        assert_eq!(count(&work), 1);
        assert_eq!(
            std::fs::metadata(&standup).unwrap().modified().unwrap(),
            before
        );

        // A calendar that went away takes its collection with it; other tools' stay.
        sync(&dir, &BTreeMap::new()).unwrap();
        assert!(!work.exists());
        assert_eq!(
            std::fs::read_to_string(dir.join("khal/mine.ics")).unwrap(),
            "theirs"
        );

        // A directory in the way that is not ours is never taken over.
        std::fs::create_dir_all(&work).unwrap();
        assert_eq!(
            sync(&dir, &first).unwrap_err().kind(),
            io::ErrorKind::AlreadyExists
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
