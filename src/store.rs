//! Everything the UI knows, built only from [`SyncEvent`]s. Wrapped in a GPUI entity by the app
//! so that every window, the tray and the reminder timer read the same state.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::Instant;

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;

use crate::config::Prefs;
use crate::model;
use crate::model::{Calendar, Event, Response};
use crate::sync::{AuthState, Command, SyncEvent, SyncHandle};

/// The account data source represented by a [`Store`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Calendars and events synced from Google.
    Google,
    /// Generated local sample calendars and events.
    Demo,
}

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
    /// Transient notice and the instant at which it should expire.
    /// Account that needs RSVP authorization again after a missing-scope response.
    pub rsvp_reauth: Option<String>,
    pub notice: Option<(String, Instant)>,
    /// User preferences applied to the current UI state.
    pub prefs: Prefs,
    pending_replies: HashMap<(String, String), (Response, Response)>,
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
        }
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
                let previous = self.prefs.rsvp_accounts.len();
                self.prefs
                    .rsvp_accounts
                    .retain(|email| signed_in.contains(email.as_str()));
                if self.prefs.rsvp_accounts.len() != previous {
                    self.prefs.save();
                }
                self.accounts = accounts;
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
                let events: Vec<Event> = events
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
                self.chunks
                    .entry(calendar_id)
                    .or_default()
                    .insert(month, events);
            }
            SyncEvent::Busy(b) => self.busy = b,
            SyncEvent::Synced(t) => self.last_sync = Some(t),
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
        let (tx, _rx) = std::sync::mpsc::channel();
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
        s
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
}
