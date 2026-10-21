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
    pub calendars: Vec<Calendar>,
    chunks: HashMap<(String, Date), Vec<Event>>,
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
                if matches!(auth, AuthState::SignedOut | AuthState::NeedsClient) {
                    self.calendars.clear();
                    self.chunks.clear();
                    self.last_sync = None;
                }
                self.auth = auth;
            }
            SyncEvent::Client(id) => self.client_id = id,
            SyncEvent::Calendars(cals) => {
                let ids: HashSet<&str> = cals.iter().map(|c| c.id.as_str()).collect();
                self.chunks.retain(|(cal, _), _| ids.contains(cal.as_str()));
                self.calendars = cals;
            }
            SyncEvent::Month {
                calendar_id,
                month,
                events,
            } => {
                self.chunks.insert((calendar_id, month), events);
            }
            SyncEvent::Busy(b) => self.busy = b,
            SyncEvent::Synced(t) => self.last_sync = Some(t),
            SyncEvent::Problem(p) => self.problem = p,
            SyncEvent::Notice(n) => self.notice = Some((n, Instant::now())),
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
        self.chunks.keys().any(|(_, m)| *m == month)
    }

    /// Events on visible calendars overlapping `[from, to)`, de-duplicated (an event spanning
    /// months is in several chunks) and sorted all-day first, then by start.
    pub fn events_between(&self, from: Date, to: Date, tz: &TimeZone) -> Vec<Event> {
        let months = months_between(from, to);
        let mut seen = HashSet::new();
        let mut out = vec![];
        for cal in self.calendars.iter().filter(|c| self.is_visible(c)) {
            for m in &months {
                let Some(events) = self.chunks.get(&(cal.id.clone(), *m)) else {
                    continue;
                };
                for e in events {
                    if !seen.insert((e.calendar_id.as_str(), e.id.as_str())) {
                        continue;
                    }
                    let mut d = from;
                    while d < to {
                        if e.occurs_on(d, tz) {
                            out.push(e.clone());
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

    pub fn events_on(&self, date: Date, tz: &TimeZone) -> Vec<Event> {
        self.events_between(date, add_days(date, 1), tz)
    }

    pub fn signed_in(&self) -> bool {
        matches!(self.auth, AuthState::SignedIn { .. } | AuthState::Demo)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::When;
    use jiff::civil::date;

    fn store() -> Store {
        let (tx, _rx) = std::sync::mpsc::channel();
        let mut s = Store::new(SyncHandle::from_sender(tx), true, Prefs::default());
        s.apply(SyncEvent::Calendars(vec![
            Calendar {
                id: "a".into(),
                name: "A".into(),
                color: 1,
                primary: true,
                default_reminders: vec![],
                selected_upstream: true,
            },
            Calendar {
                id: "b".into(),
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
    fn sign_out_clears_data() {
        let mut s = store();
        s.apply(SyncEvent::Month {
            calendar_id: "a".into(),
            month: date(2026, 10, 1),
            events: vec![],
        });
        assert!(s.has_month(date(2026, 10, 1)));
        s.apply(SyncEvent::Auth(AuthState::SignedOut));
        assert!(s.calendars.is_empty());
        assert!(!s.has_month(date(2026, 10, 1)));
    }
}
