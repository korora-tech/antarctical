//! A read-only stand-in for Dux Calendar's `dankcal` daemon, so the Dux desktop shell (a
//! DankMaterialShell fork) shows Antarctical's events in its bar, dashboard and calendar.
//!
//! Dux finds the daemon at `$DANKCAL_SOCKET`, or at `$XDG_RUNTIME_DIR/dankcal-<pid>.sock` while
//! `<pid>` is alive, and speaks newline-delimited JSON over two connections: requests
//! (`{"id","method","params"}` → `{"id","result"}` or `{"id","error"}`) and a `subscribe`d one
//! that receives pushes (`{"event":"events"}`) and then reloads. Antarctical answers
//! `calendars.list`, `events.list` and `tasks.list` (always empty); every write is refused.
//!
//! The same socket makes Antarctical single-instance: a second launch finds it, recognises it
//! with the private `antarctical.hello`, asks it to `antarctical.show` and exits.
//!
//! Plain threads (one to accept, one per connection) read a [`Snapshot`] that the UI thread
//! publishes whenever the store changes.

use std::collections::BTreeSet;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
use serde::Serialize;
use serde_json::{Value, json};

use crate::model::{Calendar, Event, Response, When, add_days, months_between};
use crate::store::Store;
use crate::sync::{Command, SyncHandle, ViewId};

/// Longest request line accepted; Dux's requests are a few hundred bytes.
const MAX_LINE: usize = 64 * 1024;
/// Dux holds two connections; a restarting shell briefly holds four.
const MAX_CONNECTIONS: usize = 16;
/// Never fetch more than this many months for the shell (Dux asks for about five).
const MAX_SHELL_MONTHS: usize = 12;
const IO_TIMEOUT: Duration = Duration::from_secs(2);
/// Calendar id suffix for the synthetic "declined" calendar of each calendar.
pub const DECLINED_SUFFIX: &str = "#declined";
const DECLINED_COLOR: &str = "#9e9e9e";

const WRITE_METHODS: &[&str] = &[
    "events.create",
    "events.update",
    "events.delete",
    "tasks.create",
    "tasks.update",
    "tasks.complete",
    "tasks.delete",
];

/// What a socket client asks of the UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellAction {
    /// Another launch of Antarctical: bring the main window forward.
    Show,
}

// ----- snapshot -------------------------------------------------------------------------------

/// Everything the socket serves, built on the UI thread from the store.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub calendars: Vec<WireCalendar>,
    /// Sorted by start, then id.
    pub events: Vec<WireEvent>,
}

/// An entry of `calendars.list`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireCalendar {
    pub id: String,
    pub name: String,
    /// `#rrggbb`.
    pub color: String,
    pub read_only: bool,
    /// Hidden in Antarctical's sidebar; Dux leaves these out.
    pub hidden: bool,
    pub account_name: String,
    pub account_id: String,
    pub holds_tasks: bool,
}

/// An entry of `events.list`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WireEvent {
    /// The event's instants (all-day events in the local zone), for range queries.
    #[serde(skip)]
    pub from: Timestamp,
    #[serde(skip)]
    pub to: Timestamp,
    pub id: String,
    pub calendar_id: String,
    pub summary: String,
    pub description: String,
    pub location: String,
    pub url: String,
    pub meeting_url: String,
    /// RFC 3339 in UTC. All-day events give their dates at UTC midnight, which is how Dux reads
    /// them (`_dayBoundary`); the end is exclusive.
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub status: &'static str,
    pub recurring_id: String,
    pub attendees: Vec<WireAttendee>,
    pub organizer: Option<WirePerson>,
    pub reminders: Vec<WireReminder>,
}

/// Dux does not show attendees yet; these follow the event's own shape.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WireAttendee {
    pub name: String,
    pub email: String,
    /// `accepted`, `tentative`, `declined` or `needs-action`.
    pub status: &'static str,
    pub organizer: bool,
    #[serde(rename = "self")]
    pub is_self: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WirePerson {
    pub name: String,
    pub email: String,
}

/// The shape Dux's event editor reads (`reminders[0].minutes`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct WireReminder {
    pub method: &'static str,
    pub minutes: i64,
}

/// The id of `calendar_id`'s synthetic calendar for declined events.
pub fn declined_id(calendar_id: &str) -> String {
    format!("{calendar_id}{DECLINED_SUFFIX}")
}

/// Builds what the socket serves from the store. Events of hidden calendars are included (the
/// calendar says `hidden`, and Dux leaves them out). Declined events are left out unless
/// [`Prefs::show_declined`](crate::config::Prefs::show_declined) is on; then they are listed
/// under a synthetic `<calendar id>#declined` calendar, which is only listed while it has events
/// (otherwise every calendar would appear twice in Dux).
pub fn snapshot(store: &Store, tz: &TimeZone) -> Snapshot {
    let show_declined = store.prefs.show_declined;
    let mut with_declined = BTreeSet::new();
    let mut events = vec![];
    for (cal, e) in store.loaded_events() {
        let declined = e.declined();
        if declined && !show_declined {
            continue;
        }
        let calendar_id = if declined {
            with_declined.insert(cal.id.as_str());
            declined_id(&cal.id)
        } else {
            cal.id.clone()
        };
        events.push(wire_event(cal, e, calendar_id, tz));
    }
    events.sort_by(|a, b| a.from.cmp(&b.from).then_with(|| a.id.cmp(&b.id)));
    let mut calendars = vec![];
    for cal in &store.calendars {
        let hidden = !store.is_visible(cal);
        calendars.push(WireCalendar {
            id: cal.id.clone(),
            name: cal.name.clone(),
            color: format!("#{:06x}", cal.color),
            read_only: true,
            hidden,
            account_name: cal.account.clone(),
            account_id: cal.account.clone(),
            holds_tasks: false,
        });
        if with_declined.contains(cal.id.as_str()) {
            calendars.push(WireCalendar {
                id: declined_id(&cal.id),
                name: format!("{} (declined)", cal.name),
                color: DECLINED_COLOR.into(),
                read_only: true,
                hidden,
                account_name: cal.account.clone(),
                account_id: cal.account.clone(),
                holds_tasks: false,
            });
        }
    }
    Snapshot { calendars, events }
}

fn wire_event(cal: &Calendar, e: &Event, calendar_id: String, tz: &TimeZone) -> WireEvent {
    let span = e.instants(tz);
    let from = span.start;
    let to = span.end;
    let (start, end, all_day) = match e.when {
        When::Timed { start, end } => (utc(start), utc(end.max(start)), false),
        When::AllDay { start, end } => {
            let end = end.max(add_days(start, 1));
            (
                format!("{start}T00:00:00Z"),
                format!("{end}T00:00:00Z"),
                true,
            )
        }
    };
    let organizer = e.organizer.as_ref().map(|name| WirePerson {
        name: name.clone(),
        email: e
            .attendees
            .iter()
            .find(|a| a.organizer)
            .map(|a| a.email.clone())
            .unwrap_or_default(),
    });
    WireEvent {
        from,
        to,
        id: wire_id(&cal.remote_id, &e.id),
        calendar_id,
        summary: e.title.clone(),
        description: e.description.clone().unwrap_or_default(),
        location: e.location.clone().unwrap_or_default(),
        url: e.html_link.clone().unwrap_or_default(),
        meeting_url: e.meet_link.clone().unwrap_or_default(),
        start,
        end,
        all_day,
        status: if e.tentative {
            "tentative"
        } else {
            "confirmed"
        },
        recurring_id: String::new(),
        attendees: e
            .attendees
            .iter()
            .map(|a| WireAttendee {
                name: a.name.clone(),
                email: a.email.clone(),
                status: match a.response {
                    Response::Accepted => "accepted",
                    Response::Tentative => "tentative",
                    Response::Declined => "declined",
                    Response::NeedsAction => "needs-action",
                },
                organizer: a.organizer,
                is_self: a.is_self,
            })
            .collect(),
        organizer,
        reminders: e
            .reminders
            .iter()
            .map(|&minutes| WireReminder {
                method: "popup",
                minutes,
            })
            .collect(),
    }
}

/// Unique across calendars (the same meeting sits in several). Dux treats ids starting with
/// `task_` or `vtodo_` as tasks, so those are prefixed.
fn wire_id(remote_id: &str, event_id: &str) -> String {
    let id = format!("{remote_id}/{event_id}");
    if id.starts_with("task_") || id.starts_with("vtodo_") {
        format!("event:{id}")
    } else {
        id
    }
}

fn utc(t: Timestamp) -> String {
    t.strftime("%Y-%m-%dT%H:%M:%SZ").to_string()
}

// ----- claiming the socket ----------------------------------------------------------------

/// The bound socket, ready for [`Server::start`].
pub struct Listener {
    listener: UnixListener,
    path: PathBuf,
    /// (device, inode) of the socket file, so quitting never removes someone else's.
    file: (u64, u64),
}

impl Listener {
    /// Removes the socket file without serving it (the app failed to start).
    pub fn discard(self) {
        if std::fs::metadata(&self.path).is_ok_and(|m| (m.dev(), m.ino()) == self.file) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

pub enum Claim {
    Bound(Listener),
    /// Another Antarctical is running and was asked to show its window (unless hidden).
    HandedOff,
    /// Running without the socket (no private runtime directory, a dankcal daemon owns
    /// `$DANKCAL_SOCKET`, or binding failed).
    Unavailable(String),
}

/// Held for the life of the process by whichever launch owns the socket.
static LOCK: std::sync::OnceLock<std::fs::File> = std::sync::OnceLock::new();

/// How long a launch that finds the lock taken waits for the first one to answer.
const HAND_OFF_WAIT: Duration = Duration::from_secs(3);

/// Finds a running Antarctical (and hands over to it) or binds the socket. `show`: ask the
/// running instance to show its window; a second `--hidden` launch just exits.
///
/// Launches serialise on an exclusive lock (`antarctical.lock` beside the socket): the holder
/// cleans up stale sockets and binds; a launch that finds it taken only looks for the holder's
/// socket, retrying briefly while the holder binds.
pub fn claim(show: bool) -> Claim {
    let explicit = std::env::var_os("DANKCAL_SOCKET")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let runtime = runtime_dir();
    let lock_path = match (&runtime, &explicit) {
        (Some(dir), _) => dir.join("antarctical.lock"),
        (None, Some(p)) => p.with_extension("lock"),
        (None, None) => {
            return Claim::Unavailable("XDG_RUNTIME_DIR is unset or not private".into());
        }
    };
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(&lock_path);
    let lock = match lock {
        Ok(f) => f,
        Err(e) => {
            return Claim::Unavailable(format!("cannot open {}: {e}", lock_path.display()));
        }
    };
    let candidates = || {
        let mut c: Vec<PathBuf> = explicit.iter().cloned().collect();
        if let Some(dir) = &runtime {
            c.extend(pid_sockets(dir));
        }
        c
    };
    match lock.try_lock() {
        Ok(()) => {
            let _ = LOCK.set(lock);
        }
        Err(std::fs::TryLockError::WouldBlock) => {
            let deadline = std::time::Instant::now() + HAND_OFF_WAIT;
            loop {
                if let Some(claim) = find_running(&candidates(), explicit.as_ref(), show) {
                    return claim;
                }
                if std::time::Instant::now() >= deadline {
                    return Claim::Unavailable(
                        "another Antarctical holds the lock but does not answer".into(),
                    );
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
        Err(std::fs::TryLockError::Error(e)) => {
            return Claim::Unavailable(format!("cannot lock {}: {e}", lock_path.display()));
        }
    }
    // Holding the lock: no other launch is binding, so a socket nobody listens on is stale.
    if let Some(dir) = &runtime {
        remove_stale(dir);
    }
    if let Some(claim) = find_running(&candidates(), explicit.as_ref(), show) {
        return claim;
    }
    let path = match (explicit, runtime) {
        (Some(p), _) => p,
        (None, Some(dir)) => dir.join(format!("dankcal-{}.sock", std::process::id())),
        (None, None) => unreachable!("no lock path without either"),
    };
    match bind(&path) {
        Ok(l) => Claim::Bound(l),
        Err(e) => Claim::Unavailable(format!("cannot listen on {}: {e}", path.display())),
    }
}

/// Probes `candidates`: hands over to a running Antarctical, or gives up when a dankcal daemon
/// owns `$DANKCAL_SOCKET`. `None`: carry on.
fn find_running(candidates: &[PathBuf], explicit: Option<&PathBuf>, show: bool) -> Option<Claim> {
    for path in candidates {
        match probe(path) {
            Probe::Antarctical(mut conn, pid) => {
                // One line per launch: a `--hidden` service retrying while another instance
                // runs logs just this.
                eprintln!("antarctical: already running (pid {pid}); handing over");
                if show && let Err(e) = conn.call("antarctical.show") {
                    eprintln!("antarctical: the running instance did not answer: {e}");
                }
                return Some(Claim::HandedOff);
            }
            Probe::Other if Some(path) == explicit => {
                return Some(Claim::Unavailable(format!(
                    "a dankcal daemon already listens on {}",
                    path.display()
                )));
            }
            Probe::Other => eprintln!(
                "antarctical: a dankcal daemon also listens on {}; Dux may connect to either",
                path.display()
            ),
            Probe::Dead => {}
        }
    }
    None
}

/// `$XDG_RUNTIME_DIR` when it is ours and private (0700), as the spec requires.
fn runtime_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    let meta = std::fs::metadata(&dir).ok()?;
    let uid = std::fs::metadata("/proc/self").ok()?.uid();
    (dir.is_absolute() && meta.is_dir() && meta.uid() == uid && meta.mode() & 0o077 == 0)
        .then_some(dir)
}

/// `dankcal-<pid>.sock` sockets in `dir`, other than ours.
fn pid_sockets(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let ours = format!("dankcal-{}.sock", std::process::id());
    entries
        .flatten()
        .filter(|entry| {
            let name = entry.file_name();
            let is_pid_socket = name
                .to_str()
                .and_then(|n| n.strip_prefix("dankcal-"))
                .and_then(|n| n.strip_suffix(".sock"))
                .is_some_and(|n| n.parse::<u32>().is_ok());
            is_pid_socket
                && name.to_str() != Some(ours.as_str())
                && entry.file_type().is_ok_and(|t| t.is_socket())
        })
        .map(|entry| entry.path())
        .collect()
}

/// Removes `dankcal-<pid>.sock` sockets nobody listens on, whatever process now has that pid.
/// Only safe while holding the lock (another launch could be between bind and listen).
fn remove_stale(dir: &Path) {
    for path in pid_sockets(dir) {
        if matches!(UnixStream::connect(&path), Err(e) if e.kind() == io::ErrorKind::ConnectionRefused)
        {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Binds `path` (0600), replacing a stale socket nobody listens on.
pub(crate) fn bind(path: &Path) -> io::Result<Listener> {
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        if !meta.file_type().is_socket() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "a file that is not a socket is in the way",
            ));
        }
        if UnixStream::connect(path).is_ok() {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                "something already listens there",
            ));
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    let meta = std::fs::metadata(path)?;
    Ok(Listener {
        listener,
        path: path.to_path_buf(),
        file: (meta.dev(), meta.ino()),
    })
}

enum Probe {
    /// A running Antarctical, and its pid.
    Antarctical(Client, u64),
    /// Something answers that is not Antarctical (the real dankcal daemon).
    Other,
    Dead,
}

/// A blocking request connection, for probing and handing over.
struct Client {
    writer: UnixStream,
    reader: BufReader<UnixStream>,
    next_id: u64,
}

impl Client {
    fn connect(path: &Path) -> io::Result<Self> {
        let stream = UnixStream::connect(path)?;
        stream.set_read_timeout(Some(IO_TIMEOUT))?;
        stream.set_write_timeout(Some(IO_TIMEOUT))?;
        Ok(Self {
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
            next_id: 0,
        })
    }

    fn call(&mut self, method: &str) -> io::Result<Value> {
        self.next_id += 1;
        let mut line = json!({"id": self.next_id, "method": method}).to_string();
        line.push('\n');
        self.writer.write_all(line.as_bytes())?;
        let mut buf = vec![];
        match read_line(&mut self.reader, &mut buf)? {
            Line::Text => serde_json::from_slice(&buf).map_err(io::Error::other),
            Line::Eof | Line::TooLong => Err(io::ErrorKind::UnexpectedEof.into()),
        }
    }
}

fn probe(path: &Path) -> Probe {
    let Ok(mut client) = Client::connect(path) else {
        return Probe::Dead;
    };
    match client.call("antarctical.hello") {
        Ok(v) if v["result"]["app"] == "antarctical" => {
            let pid = v["result"]["pid"].as_u64().unwrap_or_default();
            Probe::Antarctical(client, pid)
        }
        Ok(_) => Probe::Other,
        Err(_) => Probe::Dead,
    }
}

// ----- serving ----------------------------------------------------------------------------

/// Cheap to clone. Dropping it does not stop serving; [`Server::shutdown`] removes the socket.
#[derive(Clone)]
pub struct Server {
    shared: Arc<Shared>,
}

struct Shared {
    snapshot: Mutex<Snapshot>,
    subscribers: Mutex<Vec<Subscriber>>,
    connections: AtomicUsize,
    next_conn: AtomicU64,
    actions: async_channel::Sender<ShellAction>,
    /// The sync worker, and the months last asked of it.
    want: Mutex<(SyncHandle, Vec<Date>)>,
    tz: TimeZone,
    path: PathBuf,
    file: (u64, u64),
    stopped: AtomicBool,
}

struct Subscriber {
    conn: u64,
    writer: Arc<Mutex<UnixStream>>,
    /// Empty: every topic.
    topics: Vec<String>,
}

impl Server {
    /// Serves `listener` on its own threads. `tz` places all-day events in time for range
    /// queries (the system zone outside tests).
    pub fn start(
        listener: Listener,
        sync: SyncHandle,
        actions: async_channel::Sender<ShellAction>,
        tz: TimeZone,
    ) -> Self {
        let Listener {
            listener,
            path,
            file,
        } = listener;
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(Snapshot::default()),
            subscribers: Mutex::new(vec![]),
            connections: AtomicUsize::new(0),
            next_conn: AtomicU64::new(0),
            actions,
            want: Mutex::new((sync, vec![])),
            tz,
            path,
            file,
            stopped: AtomicBool::new(false),
        });
        let accept = shared.clone();
        std::thread::Builder::new()
            .name("antarctical-dankcal".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if accept.stopped.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    // Over the cap the connection is simply closed.
                    if accept.connections.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                        accept.connections.fetch_sub(1, Ordering::SeqCst);
                        continue;
                    }
                    let shared = accept.clone();
                    let spawned = std::thread::Builder::new()
                        .name("antarctical-dankcal-conn".into())
                        .spawn(move || {
                            let _ = serve(&shared, stream);
                            shared.disconnected();
                        });
                    if spawned.is_err() {
                        accept.connections.fetch_sub(1, Ordering::SeqCst);
                    }
                }
            })
            .expect("spawn dankcal listener");
        Self { shared }
    }

    /// Replaces the snapshot and returns the topics to push (nothing when it is unchanged).
    /// Cheap; [`Server::push`] does the socket writes and belongs off the UI thread.
    pub fn publish(&self, next: Snapshot) -> Vec<&'static str> {
        let mut current = self.shared.snapshot.lock().expect("snapshot lock");
        let mut topics = vec![];
        if current.calendars != next.calendars {
            topics.push("calendars");
        }
        if current.events != next.events {
            topics.push("events");
        }
        *current = next;
        topics
    }

    /// Tells subscribers what changed; Dux then reloads it. A subscriber that cannot be
    /// written to (gone, or not reading for [`IO_TIMEOUT`]) is disconnected: Dux then
    /// reconnects and subscribes afresh, rather than missing pushes after a partial line.
    pub fn push(&self, topics: &[&str]) {
        let mut subs = self.shared.subscribers.lock().expect("subscribers lock");
        subs.retain(|s| {
            let ok = topics
                .iter()
                .filter(|t| s.topics.is_empty() || s.topics.iter().any(|x| x == *t))
                .all(|t| write_line(&s.writer, &json!({ "event": t })).is_ok());
            if !ok {
                let _ = s
                    .writer
                    .lock()
                    .expect("writer lock")
                    .shutdown(std::net::Shutdown::Both);
            }
            ok
        });
    }

    /// Removes the socket file (if it is still ours) and refuses new connections.
    pub fn shutdown(&self) {
        self.shared.stopped.store(true, Ordering::Relaxed);
        if std::fs::metadata(&self.shared.path)
            .is_ok_and(|m| (m.dev(), m.ino()) == self.shared.file)
        {
            let _ = std::fs::remove_file(&self.shared.path);
        }
    }
}

impl Shared {
    /// With the last client gone, the shell's months are no longer kept fresh.
    fn disconnected(&self) {
        if self.connections.fetch_sub(1, Ordering::SeqCst) == 1 {
            let mut want = self.want.lock().expect("want lock");
            if !want.1.is_empty() {
                want.1.clear();
                want.0.send(Command::Forget(ViewId::Shell));
            }
        }
    }

    fn unsubscribe(&self, conn: u64) {
        self.subscribers
            .lock()
            .expect("subscribers lock")
            .retain(|s| s.conn != conn);
    }

    /// Asks the sync worker to keep the months of `[from, to)` fresh, when they changed.
    fn want_range(&self, from: Timestamp, to: Timestamp) {
        let from = from.to_zoned(self.tz.clone()).date();
        let to = add_days(to.to_zoned(self.tz.clone()).date(), 1);
        // Refuse long spans before listing their months.
        if to <= from
            || from
                .until(to)
                .map_or(true, |s| s.get_days() > 31 * MAX_SHELL_MONTHS as i32)
        {
            return;
        }
        let months = months_between(from, to);
        if months.len() > MAX_SHELL_MONTHS {
            return;
        }
        let mut want = self.want.lock().expect("want lock");
        if want.1 != months {
            want.1 = months.clone();
            want.0.send(Command::Want(ViewId::Shell, months));
        }
    }
}

enum Line {
    Text,
    Eof,
    TooLong,
}

/// Reads one line (without its newline) into `buf`, refusing lines over [`MAX_LINE`].
fn read_line(reader: &mut impl BufRead, buf: &mut Vec<u8>) -> io::Result<Line> {
    buf.clear();
    let n = reader
        .by_ref()
        .take(MAX_LINE as u64 + 1)
        .read_until(b'\n', buf)?;
    if n == 0 {
        return Ok(Line::Eof);
    }
    if buf.last() == Some(&b'\n') {
        buf.pop();
    } else if buf.len() > MAX_LINE {
        return Ok(Line::TooLong);
    }
    Ok(Line::Text)
}

fn write_line(writer: &Mutex<UnixStream>, v: &Value) -> io::Result<()> {
    let mut line = serde_json::to_vec(v).map_err(io::Error::other)?;
    line.push(b'\n');
    writer.lock().expect("writer lock").write_all(&line)
}

fn serve(shared: &Shared, stream: UnixStream) -> io::Result<()> {
    let conn = shared.next_conn.fetch_add(1, Ordering::Relaxed);
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let writer = Arc::new(Mutex::new(stream.try_clone()?));
    let mut reader = BufReader::new(stream);
    let mut buf = vec![];
    let result = loop {
        match read_line(&mut reader, &mut buf) {
            Ok(Line::Text) => {}
            Ok(Line::Eof) => break Ok(()),
            Ok(Line::TooLong) => {
                let _ = write_line(&writer, &json!({"id": null, "error": "request too long"}));
                break Ok(());
            }
            Err(e) => break Err(e),
        }
        if buf.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let reply = match serde_json::from_slice::<Value>(&buf) {
            Ok(req) => respond(shared, conn, &writer, &req),
            Err(_) => json!({"id": null, "error": "invalid JSON"}),
        };
        if let Err(e) = write_line(&writer, &reply) {
            break Err(e);
        }
    };
    shared.unsubscribe(conn);
    result
}

fn respond(shared: &Shared, conn: u64, writer: &Arc<Mutex<UnixStream>>, req: &Value) -> Value {
    let id = req.get("id").cloned().unwrap_or(Value::Null);
    let params = &req["params"];
    let method = req["method"].as_str().unwrap_or_default();
    let result = match method {
        "subscribe" => {
            let topics: Vec<String> = params["topics"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.as_str().map(String::from))
                        .collect()
                })
                .unwrap_or_default();
            let mut subs = shared.subscribers.lock().expect("subscribers lock");
            subs.retain(|s| s.conn != conn);
            subs.push(Subscriber {
                conn,
                writer: writer.clone(),
                topics: topics.clone(),
            });
            Ok(json!({ "topics": topics }))
        }
        "calendars.list" => {
            let snap = shared.snapshot.lock().expect("snapshot lock");
            Ok(serde_json::to_value(&snap.calendars).unwrap_or_default())
        }
        "events.list" => Ok(events_list(shared, params)),
        "tasks.list" => Ok(json!({ "tasks": [] })),
        "antarctical.hello" => Ok(json!({
            "app": "antarctical",
            "version": env!("CARGO_PKG_VERSION"),
            "pid": std::process::id(),
        })),
        "antarctical.show" => {
            let _ = shared.actions.send_blocking(ShellAction::Show);
            Ok(json!(true))
        }
        m if WRITE_METHODS.contains(&m) => Err("read-only".to_string()),
        "" => Err("missing method".to_string()),
        m => Err(format!("unknown method: {m}")),
    };
    match result {
        Ok(result) => json!({ "id": id, "result": result }),
        Err(error) => json!({ "id": id, "error": error }),
    }
}

/// Answers at once with what is loaded in `[from, to)`; months that are not loaded are
/// requested from the sync worker and announced with an `events` push when they arrive.
fn events_list(shared: &Shared, params: &Value) -> Value {
    let parse = |key: &str| {
        params[key]
            .as_str()
            .and_then(|s| s.parse::<Timestamp>().ok())
    };
    let (from, to) = (parse("from"), parse("to"));
    let limit = params["limit"].as_u64().map_or(usize::MAX, |l| l as usize);
    if let (Some(from), Some(to)) = (from, to) {
        shared.want_range(from, to);
    }
    let snap = shared.snapshot.lock().expect("snapshot lock");
    let events: Vec<&WireEvent> = snap
        .events
        .iter()
        .filter(|e| {
            let after = from.is_none_or(|f| e.to > f || (e.from == e.to && e.from >= f));
            let before = to.is_none_or(|t| e.from < t);
            after && before
        })
        .take(limit)
        .collect();
    json!({ "events": events })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Prefs;
    use crate::model::Attendee;
    use crate::store::Mode;
    use crate::sync::SyncEvent;
    use jiff::civil::date;
    use std::sync::mpsc;

    fn calendar(account: &str, remote: &str, selected: bool) -> Calendar {
        Calendar {
            id: Calendar::key(account, remote),
            account: account.into(),
            remote_id: remote.into(),
            name: remote.into(),
            color: 0x33b679,
            primary: false,
            default_reminders: vec![],
            selected_upstream: selected,
        }
    }

    fn event(cal: &Calendar, id: &str, when: When) -> Event {
        Event {
            id: id.into(),
            calendar_id: cal.id.clone(),
            title: id.into(),
            when,
            color: None,
            location: Some("Room 4".into()),
            description: None,
            html_link: Some("https://calendar.google.com/x".into()),
            meet_link: Some("https://meet.google.com/abc".into()),
            organizer: Some("Sam".into()),
            attendees: vec![],
            reminders: vec![10],
            tentative: false,
        }
    }

    fn timed(start: &str, end: &str) -> When {
        When::Timed {
            start: start.parse().unwrap(),
            end: end.parse().unwrap(),
        }
    }

    fn declined(mut e: Event) -> Event {
        e.attendees = vec![Attendee {
            name: "Me".into(),
            email: "me@x".into(),
            response: Response::Declined,
            organizer: false,
            is_self: true,
        }];
        e
    }

    /// A store with a visible calendar "me@x/work" and a hidden one "me@x/side".
    fn store(events: Vec<Event>) -> (Store, Calendar, Calendar) {
        let (tx, _rx) = mpsc::channel();
        let prefs = Prefs {
            ephemeral: true,
            ..Prefs::default()
        };
        let mut s = Store::new(SyncHandle::from_sender(tx), Mode::Demo, prefs);
        let work = calendar("me@x", "work", true);
        let side = calendar("me@x", "side", false);
        s.apply(SyncEvent::Calendars(vec![work.clone(), side.clone()]));
        for month in [date(2026, 10, 1), date(2026, 11, 1)] {
            for cal in [&work, &side] {
                s.apply(SyncEvent::Month {
                    calendar_id: cal.id.clone(),
                    month,
                    events: events
                        .iter()
                        .filter(|e| e.calendar_id == cal.id)
                        .cloned()
                        .collect(),
                });
            }
        }
        (s, work, side)
    }

    fn to_json(e: &WireEvent) -> Value {
        serde_json::to_value(e).unwrap()
    }

    #[test]
    fn timed_event_is_utc_rfc3339() {
        let work = calendar("me@x", "work", true);
        let e = event(
            &work,
            "standup",
            timed("2026-10-21T09:30:00+05:30", "2026-10-21T09:45:00+05:30"),
        );
        let (s, _, _) = store(vec![e]);
        let snap = snapshot(&s, &TimeZone::UTC);
        let v = to_json(&snap.events[0]);
        assert_eq!(v["id"], "work/standup");
        assert_eq!(v["calendarId"], "me@x/work");
        assert_eq!(v["summary"], "standup");
        assert_eq!(v["start"], "2026-10-21T04:00:00Z");
        assert_eq!(v["end"], "2026-10-21T04:15:00Z");
        assert_eq!(v["allDay"], false);
        assert_eq!(v["status"], "confirmed");
        assert_eq!(v["meetingUrl"], "https://meet.google.com/abc");
        assert_eq!(v["url"], "https://calendar.google.com/x");
        assert_eq!(v["location"], "Room 4");
        assert_eq!(v["description"], "");
        assert_eq!(v["organizer"]["name"], "Sam");
        assert_eq!(v["reminders"], json!([{"method": "popup", "minutes": 10}]));
        assert!(v.get("from").is_none());
    }

    #[test]
    fn all_day_dates_are_utc_midnight_in_any_zone() {
        let work = calendar("me@x", "work", true);
        let e = event(
            &work,
            "holiday",
            When::AllDay {
                start: date(2026, 10, 21),
                end: date(2026, 10, 22),
            },
        );
        let (s, _, _) = store(vec![e]);
        for hours in [-8, 0, 9] {
            let tz = TimeZone::fixed(jiff::tz::offset(hours));
            let v = to_json(&snapshot(&s, &tz).events[0]);
            assert_eq!(v["start"], "2026-10-21T00:00:00Z");
            // Exclusive, as Dux expects.
            assert_eq!(v["end"], "2026-10-22T00:00:00Z");
            assert_eq!(v["allDay"], true);
        }
    }

    #[test]
    fn multi_day_event_across_months_is_listed_once() {
        let work = calendar("me@x", "work", true);
        let trip = event(
            &work,
            "trip",
            When::AllDay {
                start: date(2026, 10, 29),
                end: date(2026, 11, 3),
            },
        );
        let overnight = event(
            &work,
            "deploy",
            timed("2026-10-31T22:00:00Z", "2026-11-01T02:00:00Z"),
        );
        let (s, _, _) = store(vec![trip, overnight]);
        let snap = snapshot(&s, &TimeZone::UTC);
        assert_eq!(snap.events.len(), 2);
        let trip = snap.events.iter().find(|e| e.id == "work/trip").unwrap();
        assert_eq!(trip.end, "2026-11-03T00:00:00Z");
    }

    #[test]
    fn hidden_calendars_are_flagged_not_dropped() {
        let (s, work, side) = store(vec![]);
        let side_event = event(
            &side,
            "x",
            timed("2026-10-21T10:00:00Z", "2026-10-21T11:00:00Z"),
        );
        let (mut s2, _, _) = store(vec![side_event]);
        let snap = snapshot(&s, &TimeZone::UTC);
        assert_eq!(snap.calendars.len(), 2);
        assert!(!snap.calendars[0].hidden);
        assert!(snap.calendars[1].hidden);
        assert_eq!(snap.calendars[0].color, "#33b679");
        assert!(snap.calendars.iter().all(|c| c.read_only && !c.holds_tasks));
        assert_eq!(snapshot(&s2, &TimeZone::UTC).events.len(), 1);
        // Sidebar toggles follow.
        s2.toggle_calendar(&side.id);
        s2.toggle_calendar(&work.id);
        let snap = snapshot(&s2, &TimeZone::UTC);
        assert!(snap.calendars[0].hidden);
        assert!(!snap.calendars[1].hidden);
    }

    #[test]
    fn declined_events_are_hidden_or_routed_to_a_grey_calendar() {
        let side = calendar("me@x", "side", false);
        let work = calendar("me@x", "work", true);
        let events = vec![
            declined(event(
                &side,
                "nope",
                timed("2026-10-21T10:00:00Z", "2026-10-21T11:00:00Z"),
            )),
            event(
                &work,
                "yes",
                timed("2026-10-21T12:00:00Z", "2026-10-21T13:00:00Z"),
            ),
        ];
        let (mut s, _, _) = store(events);
        let snap = snapshot(&s, &TimeZone::UTC);
        assert_eq!(snap.events.len(), 1);
        assert_eq!(snap.calendars.len(), 2);

        s.prefs.show_declined = true;
        let snap = snapshot(&s, &TimeZone::UTC);
        assert_eq!(snap.events.len(), 2);
        let nope = snap.events.iter().find(|e| e.id == "side/nope").unwrap();
        assert_eq!(nope.calendar_id, "me@x/side#declined");
        // Only calendars with declined events get a synthetic sibling, right after them.
        let ids: Vec<&str> = snap.calendars.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["me@x/work", "me@x/side", "me@x/side#declined"]);
        let grey = &snap.calendars[2];
        assert_eq!(grey.name, "side (declined)");
        assert_eq!(grey.color, DECLINED_COLOR);
        assert!(grey.hidden, "follows its hidden parent");
    }

    #[test]
    fn ids_never_look_like_tasks() {
        assert_eq!(wire_id("task_list", "e1"), "event:task_list/e1");
        assert_eq!(wire_id("me@x", "e1"), "me@x/e1");
    }

    // ----- over a real socket -------------------------------------------------------------

    struct Harness {
        server: Server,
        path: PathBuf,
        sync_rx: mpsc::Receiver<Command>,
        actions: async_channel::Receiver<ShellAction>,
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            self.server.shutdown();
        }
    }

    fn harness(name: &str) -> Harness {
        let path = std::env::temp_dir().join(format!("actl-{}-{name}.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let listener = bind(&path).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        let (tx, sync_rx) = mpsc::channel();
        let (atx, actions) = async_channel::unbounded();
        let server = Server::start(listener, SyncHandle::from_sender(tx), atx, TimeZone::UTC);
        Harness {
            server,
            path,
            sync_rx,
            actions,
        }
    }

    fn request(client: &mut Client, method: &str, params: Value) -> Value {
        client.next_id += 1;
        let mut line =
            json!({"id": client.next_id, "method": method, "params": params}).to_string();
        line.push('\n');
        client.writer.write_all(line.as_bytes()).unwrap();
        let mut buf = vec![];
        assert!(matches!(
            read_line(&mut client.reader, &mut buf).unwrap(),
            Line::Text
        ));
        let v: Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(v["id"], client.next_id);
        v
    }

    fn sample_snapshot() -> Snapshot {
        let work = calendar("me@x", "work", true);
        let (s, _, _) = store(vec![
            event(
                &work,
                "in",
                timed("2026-10-21T10:00:00Z", "2026-10-21T11:00:00Z"),
            ),
            event(
                &work,
                "out",
                timed("2026-12-21T10:00:00Z", "2026-12-21T11:00:00Z"),
            ),
        ]);
        snapshot(&s, &TimeZone::UTC)
    }

    #[test]
    fn serves_the_dankcal_protocol() {
        let h = harness("proto");
        h.server.publish(sample_snapshot());
        let mut c = Client::connect(&h.path).unwrap();

        let cals = request(&mut c, "calendars.list", Value::Null);
        assert_eq!(cals["result"][0]["id"], "me@x/work");
        assert_eq!(cals["result"][0]["readOnly"], true);
        assert_eq!(cals["result"][1]["hidden"], true);

        // Dux's ids are large (Date.now() + n) and its timestamps carry milliseconds.
        c.next_id = 1_761_570_000_000;
        let evs = request(
            &mut c,
            "events.list",
            json!({"from": "2026-10-01T00:00:00.000Z", "to": "2026-11-15T00:00:00.000Z", "limit": 5000}),
        );
        let list = evs["result"]["events"].as_array().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["id"], "work/in");
        match h.sync_rx.recv_timeout(IO_TIMEOUT).unwrap() {
            Command::Want(ViewId::Shell, months) => {
                assert_eq!(months, vec![date(2026, 10, 1), date(2026, 11, 1)])
            }
            _ => panic!("expected Want(Shell)"),
        }
        // The same range is not asked for twice.
        request(
            &mut c,
            "events.list",
            json!({"from": "2026-10-01T00:00:00.000Z", "to": "2026-11-15T00:00:00.000Z"}),
        );
        assert!(h.sync_rx.try_recv().is_err());

        assert_eq!(
            request(&mut c, "tasks.list", json!({}))["result"],
            json!({"tasks": []})
        );
        for m in WRITE_METHODS {
            let v = request(&mut c, m, json!({"id": "work/in"}));
            assert_eq!(v["error"], "read-only");
            assert!(v.get("result").is_none());
        }
        assert_eq!(
            request(&mut c, "nope", Value::Null)["error"],
            "unknown method: nope"
        );

        assert_eq!(
            request(&mut c, "antarctical.hello", Value::Null)["result"]["app"],
            "antarctical"
        );
        request(&mut c, "antarctical.show", Value::Null);
        assert_eq!(h.actions.try_recv().unwrap(), ShellAction::Show);
    }

    #[test]
    fn far_future_ranges_answer_without_fetching() {
        let h = harness("far");
        let mut c = Client::connect(&h.path).unwrap();
        for (from, to) in [
            ("9999-12-01T00:00:00Z", "9999-12-30T00:00:00Z"),
            ("2026-10-01T00:00:00Z", "9999-12-30T00:00:00Z"),
        ] {
            let v = request(&mut c, "events.list", json!({"from": from, "to": to}));
            assert!(v["result"]["events"].is_array());
        }
        // Only the one-month range at the end of time is asked for; the long one is refused.
        match h.sync_rx.recv_timeout(IO_TIMEOUT).unwrap() {
            Command::Want(ViewId::Shell, months) => assert_eq!(months, vec![date(9999, 12, 1)]),
            _ => panic!("expected Want(Shell)"),
        }
        assert!(h.sync_rx.try_recv().is_err());
    }

    #[test]
    fn shell_months_are_released_when_the_last_client_leaves() {
        let h = harness("release");
        let mut c = Client::connect(&h.path).unwrap();
        request(
            &mut c,
            "events.list",
            json!({"from": "2026-10-01T00:00:00Z", "to": "2026-10-20T00:00:00Z"}),
        );
        assert!(
            matches!(h.sync_rx.recv_timeout(IO_TIMEOUT).unwrap(), Command::Want(ViewId::Shell, m) if m.len() == 1)
        );
        drop(c);
        match h.sync_rx.recv_timeout(IO_TIMEOUT).unwrap() {
            Command::Forget(ViewId::Shell) => {}
            _ => panic!("expected Forget(Shell)"),
        }
    }

    #[test]
    fn subscribers_hear_about_changes() {
        let h = harness("push");
        let mut sub = Client::connect(&h.path).unwrap();
        let reply = request(
            &mut sub,
            "subscribe",
            json!({"topics": ["calendars", "events", "tasks"]}),
        );
        assert!(reply.get("result").is_some());

        let topics = h.server.publish(sample_snapshot());
        assert_eq!(topics, vec!["calendars", "events"]);
        h.server.push(&topics);
        let mut buf = vec![];
        for expected in ["calendars", "events"] {
            assert!(matches!(
                read_line(&mut sub.reader, &mut buf).unwrap(),
                Line::Text
            ));
            let v: Value = serde_json::from_slice(&buf).unwrap();
            assert_eq!(v, json!({"event": expected}));
        }
        // Unchanged: nothing to push.
        assert!(h.server.publish(sample_snapshot()).is_empty());
    }

    #[test]
    fn overlong_lines_close_the_connection() {
        let h = harness("long");
        let mut c = Client::connect(&h.path).unwrap();
        c.writer.write_all(&vec![b'x'; MAX_LINE + 10]).unwrap();
        let mut buf = vec![];
        assert!(matches!(
            read_line(&mut c.reader, &mut buf).unwrap(),
            Line::Text
        ));
        let v: Value = serde_json::from_slice(&buf).unwrap();
        assert_eq!(v["error"], "request too long");
        assert!(matches!(
            read_line(&mut c.reader, &mut buf).unwrap(),
            Line::Eof
        ));
    }

    #[test]
    fn a_second_launch_recognises_the_first() {
        let h = harness("probe");
        assert!(
            matches!(probe(&h.path), Probe::Antarctical(_, pid) if pid == std::process::id() as u64)
        );
        let path = h.path.clone();
        drop(h);
        assert!(!path.exists(), "shutdown removes the socket");
        assert!(matches!(probe(&path), Probe::Dead));
    }

    #[test]
    fn a_stale_socket_is_replaced_but_a_live_one_is_not() {
        let path = std::env::temp_dir().join(format!("actl-{}-stale.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        drop(UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        let live = bind(&path).unwrap();
        assert_eq!(bind(&path).err().unwrap().kind(), io::ErrorKind::AddrInUse);
        drop(live);
        let _ = std::fs::remove_file(&path);
    }
}
