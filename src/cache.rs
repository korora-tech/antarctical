//! An offline copy of the last-synced calendars and events, so the app can draw something the
//! moment it starts instead of waiting for the network.
//!
//! The file lives in `$XDG_CACHE_HOME/antarctical/cache.json` (mode 0600). It is a snapshot, not a
//! database: the sync worker still refetches what is on screen, and the cache is replaced by
//! whatever it returns. It is deleted when the last account signs out or the user turns the
//! offline copy off.
//!
//! This is deliberately not Google sync-token based. Sync tokens cannot be combined with the
//! `timeMin`/`timeMax` window the app fetches, and with `singleEvents=false` the app would have
//! to expand recurrence rules itself.

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use jiff::Timestamp;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};

use crate::model::{Calendar, Event};

/// Bumped whenever the stored shape changes; other versions are ignored (and overwritten).
pub const VERSION: u32 = 2;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub last_sync: Option<Timestamp>,
    pub calendars: Vec<Calendar>,
    /// Calendar id → first-of-month → events overlapping that month.
    pub events: HashMap<String, BTreeMap<Date, Vec<Event>>>,
}

impl Snapshot {
    pub fn is_empty(&self) -> bool {
        self.calendars.is_empty()
    }
}

pub fn cache_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
        .map(|p| p.join("antarctical"))
}

fn default_path() -> Option<PathBuf> {
    cache_dir().map(|d| d.join("cache.json"))
}

/// Reads the snapshot; `None` when absent, unreadable or from another version.
pub fn load() -> Option<Snapshot> {
    load_from(&default_path()?)
}

pub fn load_from(path: &Path) -> Option<Snapshot> {
    let raw = std::fs::read(path).ok()?;
    match serde_json::from_slice::<Snapshot>(&raw) {
        Ok(s) if s.version == VERSION => Some(s),
        Ok(_) => None,
        Err(e) => {
            eprintln!("antarctical: ignoring unreadable {}: {e}", path.display());
            None
        }
    }
}

/// Atomically replaces the cache file, or removes it for an empty snapshot.
pub fn save_to(path: &Path, snapshot: &Snapshot) -> std::io::Result<()> {
    if snapshot.is_empty() {
        return match std::fs::remove_file(path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        };
    }
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec(snapshot).expect("a snapshot is plain JSON data");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)
}

/// Writes snapshots on a background thread. Snapshots that pile up while a write is in progress
/// are collapsed to the newest.
#[derive(Clone)]
pub struct Writer {
    tx: mpsc::Sender<Snapshot>,
}

impl Writer {
    pub fn start() -> Option<Writer> {
        let path = default_path()?;
        let (tx, rx) = mpsc::channel::<Snapshot>();
        std::thread::Builder::new()
            .name("antarctical-cache".into())
            .spawn(move || {
                while let Ok(mut latest) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() {
                        latest = newer;
                    }
                    if let Err(e) = save_to(&path, &latest) {
                        eprintln!("antarctical: could not write the offline cache: {e}");
                    }
                }
            })
            .ok()?;
        Some(Writer { tx })
    }

    pub fn save(&self, snapshot: Snapshot) {
        let _ = self.tx.send(snapshot);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::When;
    use jiff::civil::date;

    fn temp_path(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("antarctical-cache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("cache.json")
    }

    fn snapshot() -> Snapshot {
        let cal = Calendar {
            id: "me@x/me@x".into(),
            account: "me@x".into(),
            remote_id: "me@x".into(),
            name: "Me".into(),
            color: 0x123456,
            primary: true,
            default_reminders: vec![10],
            selected_upstream: true,
            writable: true,
        };
        let event = Event {
            id: "e1".into(),
            calendar_id: cal.id.clone(),
            title: "Lunch".into(),
            when: When::Timed {
                start: "2026-11-26T13:00:00Z".parse().unwrap(),
                end: "2026-11-26T14:00:00Z".parse().unwrap(),
            },
            color: Some(0xd50000),
            location: Some("Dishoom".into()),
            description: None,
            html_link: None,
            meet_link: None,
            organizer: None,
            attendees: vec![],
            reminders: vec![10],
            tentative: false,
            kind: Default::default(),
        };
        let mut months = BTreeMap::new();
        months.insert(date(2026, 11, 1), vec![event]);
        Snapshot {
            version: VERSION,
            last_sync: Some("2026-11-25T13:00:00Z".parse().unwrap()),
            events: HashMap::from([(cal.id.clone(), months)]),
            calendars: vec![cal],
        }
    }

    #[test]
    fn a_snapshot_survives_a_round_trip_through_disk() {
        let path = temp_path("roundtrip");
        save_to(&path, &snapshot()).unwrap();
        assert_eq!(load_from(&path), Some(snapshot()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(
                mode & 0o777,
                0o600,
                "calendar data must not be world-readable"
            );
        }
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn an_empty_snapshot_removes_the_file() {
        let path = temp_path("empty");
        save_to(&path, &snapshot()).unwrap();
        save_to(&path, &Snapshot::default()).unwrap();
        assert!(!path.exists());
        // Removing what is not there is fine.
        save_to(&path, &Snapshot::default()).unwrap();
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn unreadable_or_foreign_versions_are_ignored() {
        let path = temp_path("bad");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"not json").unwrap();
        assert_eq!(load_from(&path), None);
        let mut s = snapshot();
        s.version = VERSION + 1;
        std::fs::write(&path, serde_json::to_vec(&s).unwrap()).unwrap();
        assert_eq!(load_from(&path), None);
        assert_eq!(load_from(&path.with_file_name("missing.json")), None);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}
