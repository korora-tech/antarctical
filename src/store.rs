//! Everything the UI knows, built only from [`SyncEvent`]s. Wrapped in a GPUI entity by the app
//! so that every window, the tray and the reminder timer read the same state.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;

use crate::config::Prefs;
use crate::model::{Calendar, Event, add_days, months_between};
use crate::sync::{AuthState, Command, SyncEvent, SyncHandle};

pub struct Store {
    pub sync: SyncHandle,
    pub demo: bool,
    pub auth: AuthState,
    /// Saved OAuth client id (the secret never leaves the worker).
    pub client_id: Option<String>,
    /// Signed-in Google accounts (emails), in display order.
    pub accounts: Vec<String>,
    /// The consent URL while a browser sign-in is pending.
    pub signing_in: Option<String>,
    pub calendars: Vec<Calendar>,
    chunks: HashMap<String, HashMap<Date, Vec<Event>>>,
    pub busy: bool,
    pub last_sync: Option<Timestamp>,
    pub problem: Option<String>,
    pub notice: Option<(String, Instant)>,
    pub prefs: Prefs,
}

impl Store {
    pub fn new(sync: SyncHandle, demo: bool, prefs: Prefs) -> Self {
        Self {
            sync,
            demo,
            auth: AuthState::Starting,
            client_id: None,
            accounts: vec![],
            signing_in: None,
            calendars: vec![],
            chunks: HashMap::new(),
            busy: false,
            last_sync: None,
            problem: None,
            notice: None,
            prefs,
        }
    }

    pub fn send(&self, cmd: Command) {
        self.sync.send(cmd);
    }

    pub fn apply(&mut self, ev: SyncEvent) {
        match ev {
            SyncEvent::Auth(auth) => {
                if auth == AuthState::NeedsClient {
                    self.accounts.clear();
                    self.calendars.clear();
                    self.chunks.clear();
                    self.last_sync = None;
                }
                self.auth = auth;
            }
            SyncEvent::Client(id) => self.client_id = id,
            SyncEvent::Accounts(accounts) => {
                if accounts.is_empty() {
                    self.last_sync = None;
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
                self.chunks
                    .entry(calendar_id)
                    .or_default()
                    .insert(month, events);
            }
            SyncEvent::Busy(b) => self.busy = b,
            SyncEvent::Synced(t) => self.last_sync = Some(t),
            SyncEvent::Problem(p) => self.problem = p,
            SyncEvent::Notice(n) => self.notice = Some((n, Instant::now())),
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

    pub fn calendar(&self, id: &str) -> Option<&Calendar> {
        self.calendars.iter().find(|c| c.id == id)
    }

    pub fn is_visible(&self, cal: &Calendar) -> bool {
        self.prefs
            .calendar_visibility
            .get(&cal.id)
            .copied()
            .unwrap_or(cal.selected_upstream)
    }

    pub fn toggle_calendar(&mut self, id: &str) {
        let Some(cal) = self.calendar(id) else { return };
        let visible = !self.is_visible(cal);
        self.prefs
            .calendar_visibility
            .insert(id.to_string(), visible);
        self.prefs.save();
    }

    pub fn color_of(&self, e: &Event) -> u32 {
        e.color
            .or_else(|| self.calendar(&e.calendar_id).map(|c| c.color))
            .unwrap_or(0x9c83ff)
    }

    #[cfg(test)]
    pub fn has_month(&self, month: Date) -> bool {
        self.chunks
            .values()
            .any(|months| months.contains_key(&month))
    }

    /// Events on visible calendars overlapping `[from, to)`, de-duplicated (an event spanning
    /// months is in several chunks; a calendar shared with two accounts lists it twice) and
    /// sorted all-day first, then by start.
    pub fn events_between(&self, from: Date, to: Date, tz: &TimeZone) -> Vec<&Event> {
        let months = months_between(from, to);
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
                    if !seen.insert((cal.remote_id.as_str(), e.id.as_str())) {
                        continue;
                    }
                    let mut d = from;
                    while d < to {
                        if e.occurs_on(d, tz) {
                            out.push(e);
                            break;
                        }
                        d = add_days(d, 1);
                    }
                }
            }
        }
        out.sort_by(|a, b| {
            b.is_all_day()
                .cmp(&a.is_all_day())
                .then_with(|| a.instants(tz).0.cmp(&b.instants(tz).0))
                .then_with(|| a.title.cmp(&b.title))
        });
        out
    }

    pub fn events_on(&self, date: Date, tz: &TimeZone) -> Vec<&Event> {
        self.events_between(date, add_days(date, 1), tz)
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
    use crate::model::When;
    use jiff::civil::date;

    fn store() -> Store {
        let (tx, _rx) = std::sync::mpsc::channel();
        // Ephemeral: tests must never write the user's settings.json.
        let prefs = Prefs {
            ephemeral: true,
            ..Prefs::default()
        };
        let mut s = Store::new(SyncHandle::from_sender(tx), true, prefs);
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
}
