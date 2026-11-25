//! Everything the UI knows, built only from [`SyncEvent`]s. Wrapped in a GPUI entity by the app
//! so that every window, the tray and the reminder timer read the same state.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Instant;

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;

use crate::config::Prefs;
use crate::google::api::EventDraft;
use crate::model;
use crate::model::{Attendee, Calendar, Event, Response, When};
use crate::sync::{AuthState, Command, SyncEvent, SyncHandle, WriteFailure};

/// The account data source represented by a [`Store`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Calendars and events synced from Google.
    Google,
    /// Generated local sample calendars and events.
    Demo,
}

/// An event write that has been applied to the UI but not yet confirmed by Google.
struct PendingWrite {
    calendar_id: String,
    kind: WriteKind,
}

// Held in a map, one per in-flight write; the size difference does not matter.
#[allow(clippy::large_enum_variant)]
enum WriteKind {
    /// The optimistic copy, under a temporary id.
    Create(Event),
    Update {
        previous: Event,
        event: Event,
    },
    Delete(Event),
}

/// Prefix of the temporary ids given to events that Google has not confirmed yet.
const PENDING_ID_PREFIX: &str = "pending-";

/// Shared UI state assembled from sync-worker events.
pub struct Store {
    /// Handle used to send commands to the background sync worker.
    pub sync: SyncHandle,
    /// The data source shown by this store.
    pub mode: Mode,
    /// Most recent authentication state reported by the worker.
    pub auth: AuthState,
    /// Saved OAuth client id (the secret never leaves the worker).
    pub client_id: Option<String>,
    /// Signed-in Google accounts (emails), in display order.
    pub accounts: Vec<String>,
    /// The consent URL while a browser sign-in is pending.
    pub signing_in: Option<String>,
    /// Calendars currently available to the UI.
    pub calendars: Vec<Calendar>,
    chunks: HashMap<String, HashMap<Date, Vec<Event>>>,
    /// Whether a sync operation is in progress.
    pub busy: bool,
    /// Time of the most recent successful sync, if any.
    pub last_sync: Option<Timestamp>,
    /// Current sync or authentication problem shown in the UI.
    pub problem: Option<String>,
    /// Account that needs RSVP authorization again after a missing-scope response.
    pub rsvp_reauth: Option<String>,
    /// Transient notice and the instant at which it should expire.
    pub notice: Option<(String, Instant)>,
    /// User preferences applied to the current UI state.
    pub prefs: Prefs,
    pending_replies: HashMap<(String, String), (Response, Response)>,
    pending_writes: HashMap<u64, PendingWrite>,
    next_ticket: u64,
    cache: Option<crate::cache::Writer>,
}

impl Store {
    /// Creates shared UI state that will be populated from events emitted by `sync`.
    ///
    /// `mode` selects Google or local demo content; `prefs` supplies initial settings.
    pub fn new(sync: SyncHandle, mode: Mode, prefs: Prefs) -> Self {
        Self {
            sync,
            mode,
            auth: AuthState::Starting,
            client_id: None,
            accounts: vec![],
            signing_in: None,
            calendars: vec![],
            chunks: HashMap::new(),
            busy: false,
            last_sync: None,
            problem: None,
            rsvp_reauth: None,
            notice: None,
            prefs,
            pending_replies: HashMap::new(),
            pending_writes: HashMap::new(),
            next_ticket: 1,
            cache: None,
        }
    }

    /// Starts from the offline copy of the last sync (when there is one) and keeps it up to
    /// date from now on. Whether anything is written is still governed by
    /// [`Prefs::offline_cache`].
    pub fn with_cache(mut self, writer: Option<crate::cache::Writer>) -> Self {
        if self.prefs.offline_cache
            && let Some(snapshot) = crate::cache::load()
        {
            self.restore(snapshot);
        }
        self.cache = writer;
        self
    }

    fn restore(&mut self, snapshot: crate::cache::Snapshot) {
        self.last_sync = snapshot.last_sync;
        self.calendars = snapshot.calendars;
        self.chunks = snapshot
            .events
            .into_iter()
            .map(|(cal, months)| (cal, months.into_iter().collect()))
            .collect();
    }

    /// What the offline copy holds: loaded months within half a year of today, without events
    /// Google has not confirmed yet.
    fn snapshot(&self) -> crate::cache::Snapshot {
        let today = jiff::Zoned::now().date();
        let this_month = crate::model::first_of_month(today);
        let (from, to) = (
            crate::model::add_months(this_month, -6),
            crate::model::add_months(this_month, 6),
        );
        let events = self
            .chunks
            .iter()
            .map(|(cal, months)| {
                let months = months
                    .iter()
                    .filter(|(m, _)| **m >= from && **m <= to)
                    .map(|(m, events)| {
                        let confirmed = events
                            .iter()
                            .filter(|e| !e.id.starts_with(PENDING_ID_PREFIX))
                            .cloned()
                            .collect();
                        (*m, confirmed)
                    })
                    .collect();
                (cal.clone(), months)
            })
            .collect();
        crate::cache::Snapshot {
            version: crate::cache::VERSION,
            last_sync: self.last_sync,
            calendars: self.calendars.clone(),
            events,
        }
    }

    /// Writes the offline copy, or removes it when the preference is off or nobody is signed in.
    fn persist(&self) {
        let Some(writer) = &self.cache else { return };
        if self.mode != Mode::Google {
            return;
        }
        let snapshot = if self.prefs.offline_cache && !self.accounts.is_empty() {
            self.snapshot()
        } else {
            crate::cache::Snapshot::default()
        };
        writer.save(snapshot);
    }

    pub fn set_offline_cache(&mut self, on: bool) {
        self.prefs.offline_cache = on;
        self.prefs.save();
        self.persist();
    }

    /// Sends a command to the background sync worker.
    pub fn send(&self, cmd: Command) {
        self.sync.send(cmd);
    }

    /// Toggling off takes effect in the UI immediately; enabling is committed only after OAuth.
    pub fn set_reply_enabled(&mut self, email: &str, enabled: bool) {
        if !enabled && self.prefs.rsvp_accounts.remove(email) {
            self.prefs.save();
        }
        self.send(Command::SetReplyEnabled {
            email: email.to_string(),
            enabled,
        });
    }

    pub fn reply_enabled(&self, email: &str) -> bool {
        self.prefs.rsvp_accounts.contains(email)
    }

    pub fn reply_pending(&self, calendar_id: &str, event_id: &str) -> bool {
        self.pending_replies
            .contains_key(&(calendar_id.to_string(), event_id.to_string()))
    }

    /// Updates every cached month copy immediately and remembers the prior self response.
    pub fn optimistically_reply(
        &mut self,
        calendar_id: &str,
        event_id: &str,
        response: Response,
    ) -> bool {
        let key = (calendar_id.to_string(), event_id.to_string());
        if self.pending_replies.contains_key(&key) {
            return false;
        }
        let previous = self
            .chunks
            .values()
            .flat_map(|months| months.values())
            .flatten()
            .find(|event| {
                event.calendar_id == calendar_id && event.id == event_id && event.replyable()
            })
            .and_then(Event::self_response);
        let Some(previous) = previous.filter(|previous| *previous != response) else {
            return false;
        };
        for event in self
            .chunks
            .values_mut()
            .flat_map(|months| months.values_mut())
            .flatten()
            .filter(|event| {
                event.calendar_id == calendar_id && event.id == event_id && event.replyable()
            })
        {
            for attendee in event
                .attendees
                .iter_mut()
                .filter(|attendee| attendee.is_self)
            {
                attendee.response = response;
            }
        }
        self.pending_replies.insert(key, (previous, response));
        true
    }

    // ----- event writes ---------------------------------------------------------------------

    /// Whether this account may create, edit and delete events (the events scope is granted).
    pub fn can_edit(&self, account: &str) -> bool {
        self.mode == Mode::Demo || self.prefs.rsvp_accounts.contains(account)
    }

    /// Whether an event is still waiting for Google to confirm it (and so cannot be edited).
    pub fn is_pending_event(&self, event_id: &str) -> bool {
        event_id.starts_with(PENDING_ID_PREFIX)
    }

    /// The loaded copy of an event.
    pub fn find_event(&self, calendar_id: &str, event_id: &str) -> Option<&Event> {
        self.chunks
            .get(calendar_id)?
            .values()
            .flatten()
            .find(|e| e.id == event_id)
    }

    fn ticket(&mut self) -> u64 {
        let t = self.next_ticket;
        self.next_ticket += 1;
        t
    }

    /// The event as the UI would show it for `draft`: used for the optimistic copy and for
    /// previews while typing.
    pub fn event_from_draft(calendar: &Calendar, id: String, draft: &EventDraft) -> Event {
        Event {
            id,
            calendar_id: calendar.id.clone(),
            title: draft.title.clone(),
            when: draft.when,
            color: None,
            location: draft.location.clone().filter(|l| !l.is_empty()),
            description: draft.description.clone().filter(|d| !d.is_empty()),
            html_link: None,
            meet_link: None,
            organizer: None,
            attendees: draft
                .guests
                .iter()
                .map(|email| Attendee {
                    name: email.clone(),
                    email: email.clone(),
                    display_name: None,
                    comment: None,
                    additional_guests: None,
                    optional: None,
                    resource: None,
                    response: Response::NeedsAction,
                    organizer: false,
                    is_self: false,
                })
                .collect(),
            reminders: match draft.when {
                When::Timed { .. } => calendar.default_reminders.clone(),
                When::AllDay { .. } => vec![],
            },
            tentative: false,
        }
    }

    /// Removes every loaded copy of an event; returns one of them.
    fn remove_event(&mut self, calendar_id: &str, event_id: &str) -> Option<Event> {
        let mut found = None;
        if let Some(months) = self.chunks.get_mut(calendar_id) {
            for events in months.values_mut() {
                if let Some(i) = events.iter().position(|e| e.id == event_id) {
                    let e = events.remove(i);
                    found.get_or_insert(e);
                }
            }
        }
        found
    }

    /// Puts an event into every loaded month chunk it belongs in, replacing any copy.
    fn insert_event(&mut self, event: Event) {
        let tz = TimeZone::system();
        let Some(months) = self.chunks.get_mut(&event.calendar_id) else {
            return;
        };
        for (month, events) in months.iter_mut() {
            events.retain(|e| e.id != event.id);
            if event.in_month(*month, &tz) {
                events.push(event.clone());
            }
        }
    }

    /// Creates an event: it appears at once under a temporary id and is swapped for Google's
    /// copy (or removed, with a message) when the write finishes.
    pub fn create_event(&mut self, calendar: &Calendar, draft: EventDraft) {
        let ticket = self.ticket();
        let event =
            Self::event_from_draft(calendar, format!("{PENDING_ID_PREFIX}{ticket}"), &draft);
        self.insert_event(event.clone());
        self.pending_writes.insert(
            ticket,
            PendingWrite {
                calendar_id: calendar.id.clone(),
                kind: WriteKind::Create(event),
            },
        );
        self.send(Command::CreateEvent {
            ticket,
            calendar: calendar.clone(),
            draft: Box::new(draft),
        });
    }

    /// Edits an event in place. Returns false when the event is unknown or already has a write
    /// in flight.
    pub fn update_event(&mut self, calendar: &Calendar, event_id: &str, draft: EventDraft) -> bool {
        if self.is_pending_event(event_id) || self.write_in_flight(&calendar.id, event_id) {
            return false;
        }
        let Some(previous) = self.find_event(&calendar.id, event_id).cloned() else {
            return false;
        };
        let mut event = previous.clone();
        event.title = draft.title.clone();
        event.when = draft.when;
        event.location = draft.location.clone().filter(|l| !l.is_empty());
        // Like the patch sent to Google, an absent description leaves the old one alone.
        if let Some(description) = &draft.description {
            event.description = Some(description.clone()).filter(|d| !d.is_empty());
        }
        let ticket = self.ticket();
        self.insert_event(event.clone());
        self.pending_writes.insert(
            ticket,
            PendingWrite {
                calendar_id: calendar.id.clone(),
                kind: WriteKind::Update { previous, event },
            },
        );
        self.send(Command::UpdateEvent {
            ticket,
            calendar: calendar.clone(),
            event_id: event_id.to_string(),
            draft: Box::new(draft),
        });
        true
    }

    /// Deletes an event from the UI at once. Returns false when it is unknown or busy.
    pub fn delete_event(&mut self, calendar: &Calendar, event_id: &str) -> bool {
        if self.is_pending_event(event_id) || self.write_in_flight(&calendar.id, event_id) {
            return false;
        }
        let Some(event) = self.remove_event(&calendar.id, event_id) else {
            return false;
        };
        let ticket = self.ticket();
        self.pending_writes.insert(
            ticket,
            PendingWrite {
                calendar_id: calendar.id.clone(),
                kind: WriteKind::Delete(event),
            },
        );
        self.send(Command::DeleteEvent {
            ticket,
            calendar: calendar.clone(),
            event_id: event_id.to_string(),
        });
        true
    }

    fn write_in_flight(&self, calendar_id: &str, event_id: &str) -> bool {
        self.pending_writes.values().any(|w| {
            w.calendar_id == calendar_id
                && match &w.kind {
                    WriteKind::Create(e) => e.id == event_id,
                    WriteKind::Update { event, .. } => event.id == event_id,
                    WriteKind::Delete(e) => e.id == event_id,
                }
        })
    }

    fn finish_write(
        &mut self,
        ticket: u64,
        account: String,
        result: Result<Option<Box<Event>>, WriteFailure>,
    ) {
        let Some(write) = self.pending_writes.remove(&ticket) else {
            return;
        };
        let cal = write.calendar_id;
        let what = match &write.kind {
            WriteKind::Create(_) => "create the event",
            WriteKind::Update { .. } => "save the event",
            WriteKind::Delete(_) => "delete the event",
        };
        match (write.kind, result) {
            (WriteKind::Create(temp), Ok(Some(real))) => {
                self.remove_event(&cal, &temp.id);
                self.insert_event(*real);
            }
            (WriteKind::Update { previous, .. }, Ok(Some(real))) => {
                self.remove_event(&cal, &previous.id);
                self.insert_event(*real);
            }
            (_, Ok(_)) => {}
            (kind, Err(failure)) => {
                match kind {
                    WriteKind::Create(temp) => {
                        self.remove_event(&cal, &temp.id);
                    }
                    WriteKind::Update { previous, .. } => self.insert_event(previous),
                    WriteKind::Delete(event) => self.insert_event(event),
                }
                self.problem = Some(if failure.missing_scope {
                    format!(
                        "Could not {what}: {}. Sign in again to allow editing.",
                        failure.message
                    )
                } else {
                    format!("Could not {what}: {}", failure.message)
                });
                if failure.missing_scope {
                    self.rsvp_reauth = Some(account);
                }
            }
        }
    }

    /// Re-applies unconfirmed writes to a freshly fetched month so a background refresh does
    /// not make them flicker away.
    fn overlay_pending(&self, calendar_id: &str, month: Date, events: &mut Vec<Event>) {
        let tz = TimeZone::system();
        for w in self
            .pending_writes
            .values()
            .filter(|w| w.calendar_id == calendar_id)
        {
            match &w.kind {
                WriteKind::Create(e) => {
                    if e.in_month(month, &tz) {
                        events.push(e.clone());
                    }
                }
                WriteKind::Update { previous, event } => {
                    events.retain(|e| e.id != previous.id);
                    if event.in_month(month, &tz) {
                        events.push(event.clone());
                    }
                }
                WriteKind::Delete(e) => events.retain(|x| x.id != e.id),
            }
        }
    }

    fn finish_reply(
        &mut self,
        calendar_id: &str,
        event_id: &str,
        account: String,
        error: Option<String>,
        missing_scope: bool,
    ) {
        let key = (calendar_id.to_string(), event_id.to_string());
        if let Some(error) = error {
            if let Some((previous, _)) = self.pending_replies.remove(&key) {
                for event in self
                    .chunks
                    .values_mut()
                    .flat_map(|months| months.values_mut())
                    .flatten()
                    .filter(|event| {
                        event.calendar_id == calendar_id
                            && event.id == event_id
                            && event.replyable()
                    })
                {
                    for attendee in event.attendees.iter_mut().filter(|a| a.is_self) {
                        attendee.response = previous;
                    }
                }
            }
            self.problem = Some(if missing_scope {
                format!(
                    "Could not update the invitation response: {error}. Sign in again to grant RSVP access."
                )
            } else {
                format!("Could not update the invitation response: {error}")
            });
            if missing_scope {
                self.rsvp_reauth = Some(account);
            }
        } else {
            if let Some((_, response)) = self.pending_replies.remove(&key) {
                for event in self
                    .chunks
                    .values_mut()
                    .flat_map(|months| months.values_mut())
                    .flatten()
                    .filter(|event| {
                        event.calendar_id == calendar_id
                            && event.id == event_id
                            && event.replyable()
                    })
                {
                    for attendee in event.attendees.iter_mut().filter(|a| a.is_self) {
                        attendee.response = response;
                    }
                }
            }
            if self.rsvp_reauth.as_deref() == Some(account.as_str()) {
                self.rsvp_reauth = None;
            }
        }
    }

    pub fn apply(&mut self, ev: SyncEvent) {
        match ev {
            SyncEvent::Auth(auth) => self.auth = auth,
            SyncEvent::Client(id) => self.client_id = id,
            SyncEvent::Accounts(accounts) => {
                if accounts.is_empty() {
                    self.last_sync = None;
                }
                let signed_in: HashSet<&str> = accounts.iter().map(String::as_str).collect();
                // Offline data of an account that is gone must not outlive its sign-out.
                self.calendars
                    .retain(|c| signed_in.contains(c.account.as_str()));
                let kept: HashSet<&str> = self.calendars.iter().map(|c| c.id.as_str()).collect();
                self.chunks.retain(|cal, _| kept.contains(cal.as_str()));
                let previous = self.prefs.rsvp_accounts.len();
                self.prefs
                    .rsvp_accounts
                    .retain(|email| signed_in.contains(email.as_str()));
                if self.prefs.rsvp_accounts.len() != previous {
                    self.prefs.save();
                }
                self.accounts = accounts;
                self.persist();
            }
            SyncEvent::SigningIn(url) => self.signing_in = url,
            SyncEvent::Calendars(cals) => {
                let ids: HashSet<&str> = cals.iter().map(|c| c.id.as_str()).collect();
                self.chunks.retain(|cal, _| ids.contains(cal.as_str()));
                self.migrate_visibility(&cals);
                self.calendars = cals;
            }
            SyncEvent::Month {
                calendar_id,
                month,
                events,
            } => {
                let mut events: Vec<Event> = events
                    .into_iter()
                    .map(|mut event| {
                        let key = (calendar_id.clone(), event.id.clone());
                        if let Some((_, response)) = self.pending_replies.get(&key)
                            && event.replyable()
                        {
                            for attendee in event.attendees.iter_mut().filter(|a| a.is_self) {
                                attendee.response = *response;
                            }
                        }
                        event
                    })
                    .collect();
                self.overlay_pending(&calendar_id, month, &mut events);
                self.chunks
                    .entry(calendar_id)
                    .or_default()
                    .insert(month, events);
            }
            SyncEvent::Busy(b) => self.busy = b,
            SyncEvent::Synced(t) => {
                self.last_sync = Some(t);
                self.persist();
            }
            SyncEvent::Problem(p) => {
                if p.is_none() {
                    self.rsvp_reauth = None;
                }
                self.problem = p;
            }
            SyncEvent::Notice(n) => self.notice = Some((n, Instant::now())),
            SyncEvent::ReplyAccess { email, enabled } => {
                let changed = if enabled {
                    self.prefs.rsvp_accounts.insert(email.clone())
                } else {
                    if self.rsvp_reauth.as_deref() == Some(email.as_str()) {
                        self.rsvp_reauth = None;
                    }
                    self.prefs.rsvp_accounts.remove(&email)
                };
                if changed {
                    self.prefs.save();
                }
            }
            SyncEvent::WriteFinished {
                ticket,
                account,
                result,
            } => self.finish_write(ticket, account, result),
            SyncEvent::ReplyFinished {
                calendar_id,
                event_id,
                account,
                error,
                missing_scope,
            } => self.finish_reply(&calendar_id, &event_id, account, error, missing_scope),
        }
    }

    /// Visibility used to be keyed by Google's calendar id; it is now keyed by
    /// [`Calendar::key`]. Carry old choices over the first time each calendar is seen.
    fn migrate_visibility(&mut self, cals: &[Calendar]) {
        let vis = &mut self.prefs.calendar_visibility;
        let mut changed = false;
        for c in cals {
            if !vis.contains_key(&c.id)
                && let Some(&v) = vis.get(&c.remote_id)
            {
                vis.insert(c.id.clone(), v);
                changed = true;
            }
        }
        if changed {
            vis.retain(|k, _| k.contains('/'));
            self.prefs.save();
        }
    }

    /// Finds a calendar by its account-qualified id.
    pub fn calendar(&self, id: &str) -> Option<&Calendar> {
        self.calendars.iter().find(|c| c.id == id)
    }

    /// Returns the saved visibility override, or Google's initial setting if none exists.
    pub fn is_visible(&self, cal: &Calendar) -> bool {
        self.prefs
            .calendar_visibility
            .get(&cal.id)
            .copied()
            .unwrap_or(cal.selected_upstream)
    }

    /// Toggles and persists visibility for the calendar with `id`; unknown ids are ignored.
    pub fn toggle_calendar(&mut self, id: &str) {
        let Some(cal) = self.calendar(id) else { return };
        let visible = !self.is_visible(cal);
        self.prefs
            .calendar_visibility
            .insert(id.to_string(), visible);
        self.prefs.save();
    }

    /// Resolves an event override, then its calendar color, then the UI's default color.
    pub fn color_of(&self, e: &Event) -> u32 {
        e.color
            .or_else(|| self.calendar(&e.calendar_id).map(|c| c.color))
            .unwrap_or(0x9c83ff)
    }

    #[cfg(test)]
    /// Reports whether an event chunk for `month` is loaded.
    pub fn has_month(&self, month: Date) -> bool {
        self.chunks
            .values()
            .any(|months| months.contains_key(&month))
    }

    /// Events on visible calendars overlapping `[from, to)`, de-duplicated (an event spanning
    /// months is in several chunks; a calendar shared with two accounts lists it twice) and
    /// sorted all-day first, then by start. Declined events are left out unless
    /// [`Prefs::show_declined`] is on.
    pub fn events_between(&self, from: Date, to: Date, tz: &TimeZone) -> Vec<&Event> {
        let months = model::months_between(from, to);
        let mut seen = HashSet::new();
        let mut out = vec![];
        for cal in self.calendars.iter().filter(|c| self.is_visible(c)) {
            for m in &months {
                let Some(events) = self
                    .chunks
                    .get(cal.id.as_str())
                    .and_then(|months| months.get(m))
                else {
                    continue;
                };
                for e in events {
                    // Before `seen`: another account's copy of the event may not be declined.
                    if e.declined() && !self.prefs.show_declined {
                        continue;
                    }
                    if !seen.insert((cal.remote_id.as_str(), e.id.as_str())) {
                        continue;
                    }
                    let mut d = from;
                    while d < to {
                        if e.occurs_on(d, tz) {
                            out.push(e);
                            break;
                        }
                        d = model::add_days(d, 1);
                    }
                }
            }
        }
        out.sort_by(|a, b| {
            b.is_all_day()
                .cmp(&a.is_all_day())
                .then_with(|| a.instants(tz).start.cmp(&b.instants(tz).start))
                .then_with(|| a.title.cmp(&b.title))
        });
        out
    }

    /// Months (first days) with events loaded for at least one calendar.
    pub fn loaded_months(&self) -> BTreeSet<Date> {
        self.chunks
            .values()
            .flat_map(|months| months.keys().copied())
            .collect()
    }

    /// Every loaded event of every calendar, hidden or not, declined or not, each paired with
    /// the calendar it is shown under. Events are de-duplicated as in [`Store::events_between`]:
    /// of the copies of an event on a calendar shared by two accounts, one that is not declined
    /// wins, then one on a visible calendar.
    pub fn loaded_events(&self) -> Vec<(&Calendar, &Event)> {
        let mut cals: Vec<&Calendar> = self.calendars.iter().collect();
        // Stable: visible calendars first, otherwise in sidebar order.
        cals.sort_by_key(|c| !self.is_visible(c));
        let mut months: Vec<Date> = self.loaded_months().into_iter().collect();
        months.sort();
        let mut index: HashMap<(&str, &str), usize> = HashMap::new();
        let mut out: Vec<(&Calendar, &Event)> = vec![];
        for cal in cals {
            for m in &months {
                let Some(events) = self
                    .chunks
                    .get(cal.id.as_str())
                    .and_then(|months| months.get(m))
                else {
                    continue;
                };
                for e in events {
                    match index.get(&(cal.remote_id.as_str(), e.id.as_str())) {
                        // A copy you did not decline replaces a declined one.
                        Some(&i) => {
                            if out[i].1.declined() && !e.declined() {
                                out[i] = (cal, e);
                            }
                        }
                        None => {
                            index.insert((cal.remote_id.as_str(), e.id.as_str()), out.len());
                            out.push((cal, e));
                        }
                    }
                }
            }
        }
        out
    }

    /// Returns visible events occurring on `date` in `tz`.
    pub fn events_on(&self, date: Date, tz: &TimeZone) -> Vec<&Event> {
        self.events_between(date, model::add_days(date, 1), tz)
    }

    /// At least one account (or demo data) to show.
    pub fn signed_in(&self) -> bool {
        self.auth == AuthState::Demo || !self.accounts.is_empty()
    }

    /// Calendars grouped by account, in account order.
    pub fn calendars_by_account(&self) -> Vec<(&str, Vec<&Calendar>)> {
        let mut out: Vec<(&str, Vec<&Calendar>)> = vec![];
        for c in &self.calendars {
            match out.iter_mut().find(|(a, _)| *a == c.account) {
                Some((_, cals)) => cals.push(c),
                None => out.push((c.account.as_str(), vec![c])),
            }
        }
        let order = |a: &str| {
            self.accounts
                .iter()
                .position(|x| x == a)
                .unwrap_or(usize::MAX)
        };
        out.sort_by_key(|(a, _)| order(a));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Attendee, Response, When};
    use jiff::civil::date;

    fn store() -> Store {
        store_with_rx().0
    }

    fn store_with_rx() -> (Store, std::sync::mpsc::Receiver<Command>) {
        let (tx, rx) = std::sync::mpsc::channel();
        // Ephemeral: tests must never write the user's settings.json.
        let prefs = Prefs {
            ephemeral: true,
            ..Prefs::default()
        };
        let mut s = Store::new(SyncHandle::from_sender(tx), Mode::Demo, prefs);
        s.apply(SyncEvent::Calendars(vec![
            Calendar {
                id: "a".into(),
                account: "me@x".into(),
                remote_id: "a".into(),
                name: "A".into(),
                color: 1,
                primary: true,
                default_reminders: vec![],
                selected_upstream: true,
            },
            Calendar {
                id: "b".into(),
                account: "work@x".into(),
                remote_id: "b".into(),
                name: "B".into(),
                color: 2,
                primary: false,
                default_reminders: vec![],
                selected_upstream: false,
            },
        ]));
        (s, rx)
    }

    fn all_day(cal: &str, id: &str, start: Date, end: Date) -> Event {
        Event {
            id: id.into(),
            calendar_id: cal.into(),
            title: id.into(),
            when: When::AllDay { start, end },
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

    fn this_month() -> Date {
        model::first_of_month(jiff::Zoned::now().date())
    }

    #[test]
    fn the_offline_copy_leaves_out_unconfirmed_and_distant_events() {
        let mut s = store();
        let m = this_month();
        let far = model::add_months(m, 8);
        let day = m;
        s.apply(SyncEvent::Month {
            calendar_id: "a".into(),
            month: m,
            events: vec![
                all_day("a", "real", day, day.tomorrow().unwrap()),
                all_day("a", "pending-1", day, day.tomorrow().unwrap()),
            ],
        });
        s.apply(SyncEvent::Month {
            calendar_id: "a".into(),
            month: far,
            events: vec![all_day("a", "later", far, far.tomorrow().unwrap())],
        });
        let snap = s.snapshot();
        let kept: Vec<&str> = snap.events["a"][&m].iter().map(|e| e.id.as_str()).collect();
        assert_eq!(kept, ["real"]);
        assert!(!snap.events["a"].contains_key(&far));
        assert_eq!(snap.calendars.len(), 2);
    }

    #[test]
    fn restoring_a_snapshot_shows_its_events_and_signing_out_drops_them() {
        let mut s = store();
        let m = this_month();
        s.apply(SyncEvent::Month {
            calendar_id: "a".into(),
            month: m,
            events: vec![all_day("a", "real", m, m.tomorrow().unwrap())],
        });
        let snap = s.snapshot();

        let mut fresh = store_with_rx().0;
        fresh.calendars.clear();
        fresh.chunks.clear();
        fresh.restore(snap);
        assert!(fresh.has_month(m));
        assert_eq!(fresh.calendars.len(), 2);

        // Only me@x is still signed in: work@x's calendar and events go.
        fresh.apply(SyncEvent::Accounts(vec!["me@x".into()]));
        assert_eq!(fresh.calendars.len(), 1);
        assert!(fresh.has_month(m));
        fresh.apply(SyncEvent::Accounts(vec![]));
        assert!(fresh.calendars.is_empty());
        assert!(!fresh.has_month(m));
    }

    fn invitation() -> Event {
        let mut event = all_day("a", "invitation", date(2026, 10, 21), date(2026, 10, 22));
        event.attendees = vec![
            Attendee {
                name: "Me".into(),
                email: "me@x".into(),
                display_name: Some("Me".into()),
                comment: None,
                additional_guests: None,
                optional: None,
                resource: None,
                response: Response::Tentative,
                organizer: false,
                is_self: true,
            },
            Attendee {
                name: "Organizer".into(),
                email: "organizer@x".into(),
                display_name: None,
                comment: None,
                additional_guests: None,
                optional: None,
                resource: None,
                response: Response::Accepted,
                organizer: true,
                is_self: false,
            },
        ];
        event
    }

    #[test]
    fn spanning_event_is_listed_once_and_hidden_calendars_are_skipped() {
        let mut s = store();
        let trip = all_day("a", "trip", date(2026, 9, 29), date(2026, 10, 3));
        s.apply(SyncEvent::Month {
            calendar_id: "a".into(),
            month: date(2026, 9, 1),
            events: vec![trip.clone()],
        });
        s.apply(SyncEvent::Month {
            calendar_id: "a".into(),
            month: date(2026, 10, 1),
            events: vec![trip],
        });
        s.apply(SyncEvent::Month {
            calendar_id: "b".into(),
            month: date(2026, 10, 1),
            events: vec![all_day("b", "x", date(2026, 10, 1), date(2026, 10, 2))],
        });
        let tz = TimeZone::UTC;
        let evs = s.events_between(date(2026, 9, 28), date(2026, 10, 5), &tz);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].id, "trip");
        assert_eq!(s.events_on(date(2026, 10, 3), &tz).len(), 0);
    }

    #[test]
    fn events_are_borrowed_from_the_store() {
        let mut s = store();
        s.apply(SyncEvent::Month {
            calendar_id: "a".into(),
            month: date(2026, 10, 1),
            events: vec![all_day("a", "trip", date(2026, 10, 5), date(2026, 10, 6))],
        });
        let stored = s
            .chunks
            .values()
            .flat_map(|months| months.values())
            .flatten()
            .next()
            .unwrap();
        let returned = s.events_on(date(2026, 10, 5), &TimeZone::UTC);

        assert!(std::ptr::eq(stored, returned[0]));
    }

    #[test]
    fn removing_a_calendar_drops_its_events_and_groups_follow_account_order() {
        let mut s = store();
        s.apply(SyncEvent::Accounts(vec!["work@x".into(), "me@x".into()]));
        let groups: Vec<&str> = s.calendars_by_account().iter().map(|(a, _)| *a).collect();
        assert_eq!(groups, vec!["work@x", "me@x"]);
        s.apply(SyncEvent::Month {
            calendar_id: "b".into(),
            month: date(2026, 10, 1),
            events: vec![],
        });
        assert!(s.has_month(date(2026, 10, 1)));
        // work@x signs out: the worker sends the remaining calendars.
        let remaining = vec![s.calendars[0].clone()];
        s.apply(SyncEvent::Accounts(vec!["me@x".into()]));
        s.apply(SyncEvent::Calendars(remaining));
        assert!(!s.has_month(date(2026, 10, 1)));
        assert!(s.signed_in());
        s.apply(SyncEvent::Accounts(vec![]));
        assert!(!s.signed_in());
    }

    #[test]
    fn calendar_shared_by_two_accounts_shows_each_event_once() {
        let mut s = store();
        let shared = |account: &str| Calendar {
            id: Calendar::key(account, "team@group"),
            account: account.into(),
            remote_id: "team@group".into(),
            name: "Team".into(),
            color: 3,
            primary: false,
            default_reminders: vec![],
            selected_upstream: true,
        };
        s.apply(SyncEvent::Calendars(vec![shared("me@x"), shared("work@x")]));
        for account in ["me@x", "work@x"] {
            s.apply(SyncEvent::Month {
                calendar_id: Calendar::key(account, "team@group"),
                month: date(2026, 10, 1),
                events: vec![all_day(
                    &Calendar::key(account, "team@group"),
                    "standup",
                    date(2026, 10, 5),
                    date(2026, 10, 6),
                )],
            });
        }
        assert_eq!(s.events_on(date(2026, 10, 5), &TimeZone::UTC).len(), 1);
    }

    #[test]
    fn declined_events_follow_the_preference() {
        use crate::model::{Attendee, Response};
        let mut s = store();
        let mut e = all_day("a", "skipped", date(2026, 10, 5), date(2026, 10, 6));
        e.attendees = vec![Attendee {
            name: "Me".into(),
            email: "me@x".into(),
            display_name: None,
            comment: None,
            additional_guests: None,
            optional: None,
            resource: None,
            response: Response::Declined,
            organizer: false,
            is_self: true,
        }];
        s.apply(SyncEvent::Month {
            calendar_id: "a".into(),
            month: date(2026, 10, 1),
            events: vec![e],
        });
        let tz = TimeZone::UTC;
        assert!(s.events_on(date(2026, 10, 5), &tz).is_empty());
        // Still loaded (the dankcal socket decides what to do with it).
        assert_eq!(s.loaded_events().len(), 1);
        s.prefs.show_declined = true;
        assert_eq!(s.events_on(date(2026, 10, 5), &tz).len(), 1);
    }

    #[test]
    fn shared_event_prefers_the_copy_you_did_not_decline() {
        use crate::model::{Attendee, Response};
        let mut s = store();
        let shared = |account: &str| Calendar {
            id: Calendar::key(account, "team@group"),
            account: account.into(),
            remote_id: "team@group".into(),
            name: "Team".into(),
            color: 3,
            primary: false,
            default_reminders: vec![],
            selected_upstream: true,
        };
        s.apply(SyncEvent::Calendars(vec![shared("me@x"), shared("work@x")]));
        for (account, response) in [("me@x", Response::Declined), ("work@x", Response::Accepted)] {
            let id = Calendar::key(account, "team@group");
            let mut e = all_day(&id, "standup", date(2026, 10, 5), date(2026, 10, 6));
            e.attendees = vec![Attendee {
                name: account.into(),
                email: account.into(),
                display_name: None,
                comment: None,
                additional_guests: None,
                optional: None,
                resource: None,
                response,
                organizer: false,
                is_self: true,
            }];
            s.apply(SyncEvent::Month {
                calendar_id: id,
                month: date(2026, 10, 1),
                events: vec![e],
            });
        }
        let loaded = s.loaded_events();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].0.account, "work@x");
        assert!(!loaded[0].1.declined());
    }

    #[test]
    fn visibility_keyed_by_google_id_carries_over() {
        let mut s = store();
        s.prefs
            .calendar_visibility
            .insert("team@group".into(), false);
        s.apply(SyncEvent::Calendars(vec![Calendar {
            id: Calendar::key("me@x", "team@group"),
            account: "me@x".into(),
            remote_id: "team@group".into(),
            name: "Team".into(),
            color: 3,
            primary: false,
            default_reminders: vec![],
            selected_upstream: true,
        }]));
        assert!(!s.is_visible(&s.calendars[0]));
        assert!(!s.prefs.calendar_visibility.contains_key("team@group"));
    }

    #[test]
    fn failed_rsvp_rolls_back_only_the_self_response_and_offers_reauth() {
        let mut s = store();
        let event = invitation();
        s.apply(SyncEvent::Month {
            calendar_id: "a".into(),
            month: date(2026, 10, 1),
            events: vec![event],
        });

        assert!(s.optimistically_reply("a", "invitation", Response::Declined));
        let changed = &s.chunks["a"][&date(2026, 10, 1)][0];
        assert_eq!(changed.self_response(), Some(Response::Declined));
        assert_eq!(changed.attendees[1].response, Response::Accepted);

        s.apply(SyncEvent::ReplyFinished {
            calendar_id: "a".into(),
            event_id: "invitation".into(),
            account: "me@x".into(),
            error: Some("insufficient authentication scopes".into()),
            missing_scope: true,
        });

        let restored = &s.chunks["a"][&date(2026, 10, 1)][0];
        assert_eq!(restored.self_response(), Some(Response::Tentative));
        assert_eq!(restored.attendees[1].response, Response::Accepted);
        assert!(
            s.problem
                .as_deref()
                .unwrap()
                .contains("insufficient authentication scopes")
        );
        assert_eq!(s.rsvp_reauth.as_deref(), Some("me@x"));
    }

    #[test]
    fn successful_rsvp_survives_a_stale_month_replacement() {
        let mut s = store();
        s.apply(SyncEvent::Month {
            calendar_id: "a".into(),
            month: date(2026, 10, 1),
            events: vec![invitation()],
        });
        let stale = s.chunks["a"][&date(2026, 10, 1)][0].clone();

        assert!(s.optimistically_reply("a", "invitation", Response::Accepted));
        s.apply(SyncEvent::Month {
            calendar_id: "a".into(),
            month: date(2026, 10, 1),
            events: vec![stale],
        });
        assert_eq!(
            s.chunks["a"][&date(2026, 10, 1)][0].self_response(),
            Some(Response::Accepted),
            "an in-flight reply should remain visible during a stale sync replacement"
        );
        s.rsvp_reauth = Some("me@x".into());
        s.apply(SyncEvent::ReplyFinished {
            calendar_id: "a".into(),
            event_id: "invitation".into(),
            account: "me@x".into(),
            error: None,
            missing_scope: false,
        });

        assert_eq!(
            s.chunks["a"][&date(2026, 10, 1)][0].self_response(),
            Some(Response::Accepted)
        );
        assert!(!s.reply_pending("a", "invitation"));
        assert!(s.rsvp_reauth.is_none());
    }

    #[test]
    fn unrelated_rsvp_failure_does_not_clear_another_accounts_reauthorization_prompt() {
        let mut s = store();
        s.rsvp_reauth = Some("first@example.com".into());

        s.finish_reply(
            "calendar",
            "event",
            "second@example.com".into(),
            Some("temporary network error".into()),
            false,
        );

        assert_eq!(s.rsvp_reauth.as_deref(), Some("first@example.com"));
    }

    fn draft(title: &str) -> EventDraft {
        EventDraft {
            title: title.into(),
            when: When::AllDay {
                start: date(2026, 10, 21),
                end: date(2026, 10, 22),
            },
            time_zone: None,
            location: None,
            description: None,
            guests: vec![],
            meet: false,
            rrule: None,
        }
    }

    fn month_titles(s: &Store) -> Vec<String> {
        let mut v: Vec<String> = s.chunks["a"][&date(2026, 10, 1)]
            .iter()
            .map(|e| e.id.clone())
            .collect();
        v.sort();
        v
    }

    fn with_one_event() -> (Store, std::sync::mpsc::Receiver<Command>, Calendar) {
        let (mut s, rx) = store_with_rx();
        s.apply(SyncEvent::Month {
            calendar_id: "a".into(),
            month: date(2026, 10, 1),
            events: vec![all_day("a", "old", date(2026, 10, 5), date(2026, 10, 6))],
        });
        let cal = s.calendar("a").unwrap().clone();
        (s, rx, cal)
    }

    #[test]
    fn created_event_appears_at_once_and_is_replaced_by_googles_copy() {
        let (mut s, rx, cal) = with_one_event();
        s.create_event(&cal, draft("Lunch"));
        assert_eq!(month_titles(&s), vec!["old", "pending-1"]);
        let Ok(Command::CreateEvent { ticket, draft, .. }) = rx.try_recv() else {
            panic!("expected a create command")
        };
        assert_eq!(draft.title, "Lunch");

        // A background refresh that predates the write must not make it flicker away.
        s.apply(SyncEvent::Month {
            calendar_id: "a".into(),
            month: date(2026, 10, 1),
            events: vec![all_day("a", "old", date(2026, 10, 5), date(2026, 10, 6))],
        });
        assert_eq!(month_titles(&s), vec!["old", "pending-1"]);

        s.apply(SyncEvent::WriteFinished {
            ticket,
            account: "me@x".into(),
            result: Ok(Some(Box::new(all_day(
                "a",
                "real",
                date(2026, 10, 21),
                date(2026, 10, 22),
            )))),
        });
        assert_eq!(month_titles(&s), vec!["old", "real"]);
        assert!(s.problem.is_none());
    }

    #[test]
    fn failed_create_is_removed_with_a_message() {
        let (mut s, rx, cal) = with_one_event();
        s.create_event(&cal, draft("Lunch"));
        let Ok(Command::CreateEvent { ticket, .. }) = rx.try_recv() else {
            panic!()
        };
        s.apply(SyncEvent::WriteFinished {
            ticket,
            account: "me@x".into(),
            result: Err(WriteFailure {
                message: "boom".into(),
                missing_scope: true,
            }),
        });
        assert_eq!(month_titles(&s), vec!["old"]);
        assert!(
            s.problem
                .as_deref()
                .unwrap()
                .contains("create the event: boom")
        );
        assert_eq!(s.rsvp_reauth.as_deref(), Some("me@x"));
    }

    #[test]
    fn edit_applies_at_once_and_rolls_back_on_failure() {
        let (mut s, rx, cal) = with_one_event();
        assert!(s.update_event(&cal, "old", draft("Renamed")));
        assert_eq!(s.find_event("a", "old").unwrap().title, "Renamed");
        // One edit at a time per event.
        assert!(!s.update_event(&cal, "old", draft("Again")));
        let Ok(Command::UpdateEvent {
            ticket, event_id, ..
        }) = rx.try_recv()
        else {
            panic!()
        };
        assert_eq!(event_id, "old");
        s.apply(SyncEvent::WriteFinished {
            ticket,
            account: "me@x".into(),
            result: Err(WriteFailure {
                message: "nope".into(),
                missing_scope: false,
            }),
        });
        assert_eq!(s.find_event("a", "old").unwrap().title, "old");
        assert!(s.problem.as_deref().unwrap().contains("save the event"));
    }

    #[test]
    fn edit_keeps_what_the_editor_does_not_touch() {
        let (mut s, _rx, cal) = with_one_event();
        s.update_event(&cal, "old", draft("Renamed"));
        let e = s.find_event("a", "old").unwrap();
        assert_eq!(e.title, "Renamed");
        assert_eq!(
            e.when,
            When::AllDay {
                start: date(2026, 10, 21),
                end: date(2026, 10, 22)
            }
        );
        assert!(
            !s.chunks["a"][&date(2026, 10, 1)]
                .iter()
                .any(|e| e.id == "pending-1")
        );
    }

    #[test]
    fn delete_hides_at_once_survives_stale_refresh_and_restores_on_failure() {
        let (mut s, rx, cal) = with_one_event();
        assert!(s.delete_event(&cal, "old"));
        assert!(s.find_event("a", "old").is_none());
        s.apply(SyncEvent::Month {
            calendar_id: "a".into(),
            month: date(2026, 10, 1),
            events: vec![all_day("a", "old", date(2026, 10, 5), date(2026, 10, 6))],
        });
        assert!(
            s.find_event("a", "old").is_none(),
            "stale refresh resurrected it"
        );
        let Ok(Command::DeleteEvent { ticket, .. }) = rx.try_recv() else {
            panic!()
        };
        s.apply(SyncEvent::WriteFinished {
            ticket,
            account: "me@x".into(),
            result: Err(WriteFailure {
                message: "offline".into(),
                missing_scope: false,
            }),
        });
        assert!(s.find_event("a", "old").is_some());
        assert!(s.problem.as_deref().unwrap().contains("delete the event"));
    }

    #[test]
    fn events_still_waiting_for_google_cannot_be_edited() {
        let (mut s, _rx, cal) = with_one_event();
        s.create_event(&cal, draft("Lunch"));
        assert!(s.is_pending_event("pending-1"));
        assert!(!s.update_event(&cal, "pending-1", draft("x")));
        assert!(!s.delete_event(&cal, "pending-1"));
    }

    #[test]
    fn multi_month_events_land_in_every_loaded_month() {
        let (mut s, _rx, cal) = with_one_event();
        s.apply(SyncEvent::Month {
            calendar_id: "a".into(),
            month: date(2026, 11, 1),
            events: vec![],
        });
        let mut d = draft("Trip");
        d.when = When::AllDay {
            start: date(2026, 10, 30),
            end: date(2026, 11, 3),
        };
        s.create_event(&cal, d);
        assert!(
            s.chunks["a"][&date(2026, 10, 1)]
                .iter()
                .any(|e| e.id == "pending-1")
        );
        assert!(
            s.chunks["a"][&date(2026, 11, 1)]
                .iter()
                .any(|e| e.id == "pending-1")
        );
    }
}
