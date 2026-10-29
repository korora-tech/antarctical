//! `--demo` / `ANTARCTICAL_DEMO=1`: a worker that answers the same commands as the Google one with
//! generated calendars, so the UI can be run and reviewed without an OAuth client.

use std::collections::{HashMap, HashSet};
use std::sync::mpsc;

use jiff::civil::{Date, Weekday};
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan, Zoned};

use crate::model;
use crate::model::{Attendee, Calendar, Event, Response, When};
use crate::sync::{AuthState, Command, SyncEvent, SyncHandle};

pub fn start() -> std::io::Result<(SyncHandle, async_channel::Receiver<SyncEvent>)> {
    let (tx, rx) = mpsc::channel();
    let (etx, erx) = async_channel::unbounded();
    std::thread::Builder::new()
        .name("antarctical-demo".into())
        .spawn(move || run(rx, etx))?;
    Ok((SyncHandle::from_sender(tx), erx))
}

fn run(rx: mpsc::Receiver<Command>, tx: async_channel::Sender<SyncEvent>) {
    let emit = |e| {
        let _ = tx.send_blocking(e);
    };
    let tz = TimeZone::system();
    let cals = calendars();
    emit(SyncEvent::Client(Some(
        "demo.apps.googleusercontent.com".into(),
    )));
    emit(SyncEvent::Auth(AuthState::Demo));
    emit(SyncEvent::Accounts(vec![PERSONAL.into(), WORK.into()]));
    emit(SyncEvent::Calendars(cals.clone()));
    let today = Zoned::now().date();
    let mut reply_enabled = HashSet::new();
    let mut reply_overrides = HashMap::new();
    let send_months = |months: &[Date], overrides: &HashMap<(String, String), Response>| {
        for &month in months {
            for cal in &cals {
                let mut events = events_for(cal, month, today, &tz);
                apply_reply_overrides(&mut events, overrides);
                emit(SyncEvent::Month {
                    calendar_id: cal.id.clone(),
                    month,
                    events,
                });
            }
        }
        emit(SyncEvent::Synced(Timestamp::now()));
    };
    let m = today.first_of_month();
    send_months(
        &[model::add_months(m, -1), m, model::add_months(m, 1)],
        &reply_overrides,
    );
    while let Ok(cmd) = rx.recv() {
        match cmd {
            Command::Want(_, months) => send_months(&months, &reply_overrides),
            Command::Forget(_) => {}
            Command::Refresh => send_months(&[m], &reply_overrides),
            Command::SaveClient { client_id, .. } => {
                emit(SyncEvent::Client(Some(client_id)));
                emit(SyncEvent::Notice(
                    "Demo mode: the client is kept in memory only".into(),
                ));
            }
            Command::ClearClient => emit(SyncEvent::Client(None)),
            Command::AddAccount | Command::RemoveAccount(_) | Command::CancelSignIn => emit(
                SyncEvent::Notice("Demo mode: no Google account is used".into()),
            ),
            Command::SetReplyEnabled { email, enabled } => {
                if enabled {
                    reply_enabled.insert(email.clone());
                } else {
                    reply_enabled.remove(&email);
                }
                emit(SyncEvent::ReplyAccess { email, enabled });
            }
            Command::ReauthorizeRsvp(email) => {
                reply_enabled.insert(email.clone());
                emit(SyncEvent::ReplyAccess {
                    email,
                    enabled: true,
                });
            }
            Command::Reply {
                calendar,
                event,
                response,
            } => {
                let event = *event;
                let error = if !reply_enabled.contains(&calendar.account) {
                    Some("Reply to invitations is not enabled for this account".into())
                } else if event.calendar_id != calendar.id || !event.replyable() {
                    Some("This event does not have a replyable self attendee".into())
                } else {
                    reply_overrides.insert((calendar.id.clone(), event.id.clone()), response);
                    None
                };
                emit(SyncEvent::ReplyFinished {
                    calendar_id: calendar.id,
                    event_id: event.id,
                    account: calendar.account,
                    error,
                    missing_scope: false,
                });
            }
            Command::DismissProblems => emit(SyncEvent::Problem(None)),
            Command::Shutdown => break,
            Command::SignInFinished(..) => {}
        }
    }
}

fn apply_reply_overrides(events: &mut [Event], overrides: &HashMap<(String, String), Response>) {
    for event in events {
        let key = (event.calendar_id.clone(), event.id.clone());
        if let Some(response) = overrides.get(&key)
            && let Some(attendee) = event.attendees.iter_mut().find(|attendee| attendee.is_self)
        {
            attendee.response = *response;
        }
    }
}

const PERSONAL: &str = "you@example.com";
const WORK: &str = "you@work.example";

fn calendars() -> Vec<Calendar> {
    let cal = |account: &str, id: &str, name: &str, color, primary| Calendar {
        id: Calendar::key(account, id),
        account: account.into(),
        remote_id: id.into(),
        name: name.into(),
        color,
        primary,
        default_reminders: vec![10],
        selected_upstream: true,
    };
    vec![
        cal(PERSONAL, PERSONAL, PERSONAL, 0xf4511e, true),
        cal(PERSONAL, "holidays", "Holidays", 0x8e24aa, false),
        cal(WORK, WORK, WORK, 0x7986cb, true),
        cal(WORK, "team", "Platform team", 0x33b679, false),
    ]
}

fn at(date: Date, h: i8, m: i8, tz: &TimeZone) -> Timestamp {
    date.at(h, m, 0, 0)
        .to_zoned(tz.clone())
        .map(|z| z.timestamp())
        .unwrap_or(Timestamp::UNIX_EPOCH)
}

fn person(name: &str, response: Response, is_self: bool, organizer: bool) -> Attendee {
    Attendee {
        name: name.into(),
        email: format!("{}@example.com", name.to_lowercase().replace(' ', ".")),
        display_name: Some(name.into()),
        comment: None,
        additional_guests: None,
        optional: None,
        resource: None,
        response,
        organizer,
        is_self,
    }
}

fn events_for(cal: &Calendar, month: Date, today: Date, tz: &TimeZone) -> Vec<Event> {
    let mut out = vec![];
    let base = |id: String, title: &str, when| Event {
        id,
        calendar_id: cal.id.clone(),
        title: title.into(),
        when,
        color: None,
        location: None,
        description: None,
        html_link: Some("https://calendar.google.com/".into()),
        meet_link: None,
        organizer: None,
        attendees: vec![],
        reminders: cal.default_reminders.clone(),
        tentative: false,
    };
    let timed = |d: Date, (h1, m1): (i8, i8), (h2, m2): (i8, i8)| When::Timed {
        start: at(d, h1, m1, tz),
        end: at(d, h2, m2, tz),
    };
    let days = (0..month.days_in_month()).map(|i| model::add_days(month, i as i32));
    for d in days {
        let key = d.to_string();
        let wd = d.weekday();
        let weekday = !matches!(wd, Weekday::Saturday | Weekday::Sunday);
        match cal.remote_id.as_str() {
            WORK => {
                if wd == Weekday::Tuesday {
                    let mut e = base(
                        format!("1on1-{key}"),
                        "1:1 with Sam",
                        timed(d, (14, 0), (14, 30)),
                    );
                    e.meet_link = Some("https://meet.google.com/abc-defg-hij".into());
                    e.attendees = vec![
                        person("Sam Rivera", Response::Accepted, false, true),
                        person("You", Response::Accepted, true, false),
                    ];
                    e.organizer = Some("Sam Rivera".into());
                    e.description =
                        Some("Career goals\n• Q4 review prep\n• Conference talk".into());
                    out.push(e);
                }
                if wd == Weekday::Thursday {
                    let mut a = base(
                        format!("design-{key}"),
                        "Design review: sync engine",
                        timed(d, (10, 0), (11, 30)),
                    );
                    a.location = Some("Room 4 / Lovelace".into());
                    a.description = Some("Walk through the incremental sync proposal.".into());
                    out.push(a);
                    let mut b = base(
                        format!("vendor-{key}"),
                        "Vendor call",
                        timed(d, (10, 30), (11, 0)),
                    );
                    b.attendees = vec![
                        person("You", Response::Declined, true, false),
                        person("Alex Chen", Response::Accepted, false, true),
                    ];
                    out.push(b);
                    out.push(base(
                        format!("interview-{key}"),
                        "Interview loop",
                        timed(d, (11, 0), (12, 0)),
                    ));
                }
                if d == today {
                    // Something soon, so reminders and the tray agenda have content.
                    let now = Zoned::now();
                    let start = now.round(jiff::Unit::Minute).unwrap_or(now.clone()) + 15.minutes();
                    let mut e = base(
                        format!("soon-{key}"),
                        "Pairing: tray agenda",
                        When::Timed {
                            start: start.timestamp(),
                            end: (start.clone() + 45.minutes()).timestamp(),
                        },
                    );
                    e.color = Some(0xf6bf26);
                    e.meet_link = Some("https://meet.google.com/xyz-abcd-efg".into());
                    out.push(e);
                }
            }
            "team" => {
                if weekday {
                    let mut e = base(
                        format!("standup-{key}"),
                        "Standup",
                        timed(d, (9, 30), (9, 45)),
                    );
                    e.meet_link = Some("https://meet.google.com/std-upup-now".into());
                    out.push(e);
                }
                if wd == Weekday::Friday {
                    out.push(base(
                        format!("demo-{key}"),
                        "Team demos",
                        timed(d, (16, 0), (17, 0)),
                    ));
                }
                if d.day() == 12 {
                    let mut e = base(
                        format!("offsite-{key}"),
                        "Team offsite",
                        When::AllDay {
                            start: d,
                            end: model::add_days(d, 3),
                        },
                    );
                    e.location = Some("Lisbon".into());
                    e.reminders = vec![];
                    out.push(e);
                }
            }
            PERSONAL => {
                if matches!(wd, Weekday::Monday | Weekday::Wednesday | Weekday::Friday) {
                    out.push(base(
                        format!("gym-{key}"),
                        "Gym",
                        timed(d, (18, 0), (19, 0)),
                    ));
                }
                if wd == Weekday::Saturday {
                    out.push(base(
                        format!("market-{key}"),
                        "Farmers market",
                        timed(d, (10, 0), (11, 30)),
                    ));
                }
                if wd == Weekday::Sunday && d.day() <= 7 {
                    out.push(base(
                        format!("late-{key}"),
                        "Late film",
                        timed(d, (22, 30), (23, 59)),
                    ));
                }
            }
            "holidays" if d.day() == 1 => {
                let mut e = base(
                    format!("first-{key}"),
                    "First of the month",
                    When::AllDay {
                        start: d,
                        end: model::add_days(d, 1),
                    },
                );
                e.reminders = vec![];
                out.push(e);
            }
            _ => {}
        }
    }
    // Multi-month events must be present in every month they touch.
    if cal.remote_id == PERSONAL {
        let start = model::add_days(month, -3);
        let mut e = base(
            format!("trip-{month}"),
            "Trip",
            When::AllDay {
                start,
                end: model::add_days(month, 2),
            },
        );
        e.reminders = vec![];
        out.push(e);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn demo_rsvp_overrides_survive_month_regeneration_locally() {
        let tz = TimeZone::UTC;
        let work = calendars()
            .into_iter()
            .find(|calendar| calendar.remote_id == WORK)
            .unwrap();
        let mut events = events_for(&work, date(2026, 10, 1), date(2026, 10, 1), &tz);
        let event = events
            .iter()
            .find(|event| event.id == "1on1-2026-10-06")
            .unwrap();
        assert_eq!(event.self_response(), Some(Response::Accepted));
        let overrides = HashMap::from([(
            (event.calendar_id.clone(), event.id.clone()),
            Response::Declined,
        )]);

        apply_reply_overrides(&mut events, &overrides);

        assert_eq!(
            events
                .iter()
                .find(|event| event.id == "1on1-2026-10-06")
                .unwrap()
                .self_response(),
            Some(Response::Declined)
        );
    }
}
