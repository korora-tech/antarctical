//! Google Calendar API v3, read-only: `calendarList.list` and `events.list` with
//! `singleEvents=true` so Google expands recurrences for us.

use std::fmt;
use std::time::Duration;

use jiff::Timestamp;
use serde::Deserialize;

use crate::model::{
    Attendee, Calendar, Event, Response, When, google_event_color, parse_hex_color,
};

const BASE: &str = "https://www.googleapis.com/calendar/v3";
const BODY_LIMIT: u64 = 32 * 1024 * 1024;

const CALENDAR_FIELDS: &str = concat!(
    "items(id,summary,summaryOverride,backgroundColor,primary,selected,hidden,defaultReminders),",
    "nextPageToken",
);
const EVENT_FIELDS: &str = "items(id,status,summary,description,location,htmlLink,hangoutLink,\
    start,end,colorId,eventType,attendees(email,displayName,responseStatus,organizer,self),\
    organizer(email,displayName,self),reminders,conferenceData(entryPoints(entryPointType,uri))),\
    nextPageToken";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// The access token was rejected; refresh and retry once.
    Unauthorized,
    /// Access to this calendar is gone (unsubscribed, permissions changed).
    Forbidden(String),
    NotFound,
    RateLimited,
    Network(String),
    Http {
        status: u16,
        body: String,
    },
    Decode(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::Unauthorized => write!(f, "Google rejected the access token"),
            ApiError::Forbidden(m) => write!(f, "Access denied: {m}"),
            ApiError::NotFound => write!(f, "Calendar not found"),
            ApiError::RateLimited => write!(f, "Google Calendar rate limit reached"),
            ApiError::Network(m) => write!(f, "Network error: {m}"),
            ApiError::Http { status, body } => {
                write!(f, "Google Calendar returned HTTP {status}: {body}")
            }
            ApiError::Decode(m) => write!(f, "Unexpected response from Google: {m}"),
        }
    }
}

pub fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .user_agent(concat!("antarctical/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn get<T: serde::de::DeserializeOwned>(
    agent: &ureq::Agent,
    url: &url::Url,
    token: &str,
) -> Result<T, ApiError> {
    let mut resp = agent
        .get(url.as_str())
        .header("Authorization", &format!("Bearer {token}"))
        .call()
        .map_err(|e| ApiError::Network(e.to_string()))?;
    let status = resp.status().as_u16();
    let body = resp
        .body_mut()
        .with_config()
        .limit(BODY_LIMIT)
        .read_to_string()
        .map_err(|e| ApiError::Network(e.to_string()))?;
    match status {
        200 => serde_json::from_str(&body).map_err(|e| ApiError::Decode(e.to_string())),
        401 => Err(ApiError::Unauthorized),
        404 | 410 => Err(ApiError::NotFound),
        429 => Err(ApiError::RateLimited),
        403 if body.contains("rateLimitExceeded") || body.contains("RATE_LIMIT") => {
            Err(ApiError::RateLimited)
        }
        403 => Err(ApiError::Forbidden(error_message(&body))),
        _ => Err(ApiError::Http {
            status,
            body: error_message(&body),
        }),
    }
}

fn error_message(body: &str) -> String {
    #[derive(Deserialize)]
    struct Envelope {
        error: Inner,
    }
    #[derive(Deserialize)]
    struct Inner {
        message: String,
    }
    serde_json::from_str::<Envelope>(body)
        .map(|e| e.error.message)
        .unwrap_or_else(|_| body.chars().take(200).collect())
}

/// The account's email: the id of its primary calendar (available under any calendar scope,
/// so no extra identity scope is needed).
pub fn primary_email(agent: &ureq::Agent, token: &str) -> Result<String, ApiError> {
    #[derive(Deserialize)]
    struct Primary {
        id: String,
    }
    let url = url::Url::parse(&format!("{BASE}/calendars/primary?fields=id"))
        .expect("BASE and the primary-calendar path form a valid Google API URL");
    get::<Primary>(agent, &url, token).map(|p| p.id)
}

/// The calendars listed under `account`, primary first, then by name.
pub fn calendar_list(
    agent: &ureq::Agent,
    token: &str,
    account: &str,
) -> Result<Vec<Calendar>, ApiError> {
    let mut out = vec![];
    let mut page: Option<String> = None;
    loop {
        let mut url = url::Url::parse(&format!("{BASE}/users/me/calendarList"))
            .expect("BASE and the calendar-list path form a valid Google API URL");
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("maxResults", "250")
                .append_pair("fields", CALENDAR_FIELDS);
            if let Some(p) = &page {
                q.append_pair("pageToken", p);
            }
        }
        let resp: CalendarListResponse = get(agent, &url, token)?;
        out.extend(resp.items.into_iter().filter_map(|c| c.into_model(account)));
        match resp.next_page_token {
            Some(p) => page = Some(p),
            None => break,
        }
    }
    // Primary first, then by name, so the sidebar order is stable between syncs.
    out.sort_by(|a, b| {
        b.primary
            .cmp(&a.primary)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(out)
}

/// All event instances overlapping `[from, to)`.
pub fn events(
    agent: &ureq::Agent,
    token: &str,
    calendar: &Calendar,
    from: Timestamp,
    to: Timestamp,
) -> Result<Vec<Event>, ApiError> {
    let mut out = vec![];
    let mut page: Option<String> = None;
    loop {
        let mut url =
            url::Url::parse(BASE).expect("BASE is a valid static Google Calendar API URL");
        url.path_segments_mut()
            .expect("BASE has no query or fragment, so its path segments are mutable")
            .push("calendars")
            .push(&calendar.remote_id)
            .push("events");
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("singleEvents", "true")
                .append_pair("orderBy", "startTime")
                .append_pair("timeMin", &from.to_string())
                .append_pair("timeMax", &to.to_string())
                .append_pair("maxResults", "2500")
                .append_pair("fields", EVENT_FIELDS);
            if let Some(p) = &page {
                q.append_pair("pageToken", p);
            }
        }
        let resp: EventsResponse = get(agent, &url, token)?;
        out.extend(
            resp.items
                .into_iter()
                .filter_map(|e| e.into_model(calendar)),
        );
        match resp.next_page_token {
            Some(p) => page = Some(p),
            None => break,
        }
    }
    Ok(out)
}

// ----- wire types ---------------------------------------------------------------------------

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalendarListResponse {
    #[serde(default)]
    items: Vec<CalendarListEntry>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CalendarListEntry {
    id: String,
    summary: Option<String>,
    summary_override: Option<String>,
    background_color: Option<String>,
    #[serde(default)]
    primary: bool,
    #[serde(default)]
    selected: bool,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    default_reminders: Vec<Reminder>,
}

impl CalendarListEntry {
    fn into_model(self, account: &str) -> Option<Calendar> {
        if self.hidden {
            return None;
        }
        Some(Calendar {
            id: Calendar::key(account, &self.id),
            account: account.to_string(),
            name: self
                .summary_override
                .or(self.summary)
                .unwrap_or_else(|| self.id.clone()),
            color: self
                .background_color
                .as_deref()
                .and_then(parse_hex_color)
                .unwrap_or(0x9c83ff),
            primary: self.primary,
            default_reminders: popup_minutes(&self.default_reminders),
            selected_upstream: self.selected || self.primary,
            remote_id: self.id,
        })
    }
}

#[derive(Deserialize)]
struct Reminder {
    method: String,
    minutes: i64,
}

fn popup_minutes(reminders: &[Reminder]) -> Vec<i64> {
    let mut m: Vec<i64> = reminders
        .iter()
        .filter(|r| r.method == "popup" && r.minutes >= 0)
        .map(|r| r.minutes)
        .collect();
    m.sort_unstable();
    m.dedup();
    m
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct EventsResponse {
    #[serde(default)]
    items: Vec<WireEvent>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireEvent {
    id: String,
    status: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    location: Option<String>,
    html_link: Option<String>,
    hangout_link: Option<String>,
    start: Option<WireTime>,
    end: Option<WireTime>,
    color_id: Option<String>,
    event_type: Option<String>,
    #[serde(default)]
    attendees: Vec<WireAttendee>,
    organizer: Option<WirePerson>,
    reminders: Option<WireReminders>,
    conference_data: Option<WireConference>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireTime {
    date: Option<String>,
    date_time: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireAttendee {
    email: Option<String>,
    display_name: Option<String>,
    response_status: Option<String>,
    #[serde(default)]
    organizer: bool,
    #[serde(rename = "self", default)]
    is_self: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WirePerson {
    email: Option<String>,
    display_name: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireReminders {
    #[serde(default)]
    use_default: bool,
    #[serde(default)]
    overrides: Vec<Reminder>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireConference {
    #[serde(default)]
    entry_points: Vec<WireEntryPoint>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireEntryPoint {
    entry_point_type: Option<String>,
    uri: Option<String>,
}

impl WireEvent {
    fn into_model(self, calendar: &Calendar) -> Option<Event> {
        if self.status.as_deref() == Some("cancelled") {
            return None;
        }
        // Working-location markers are all-day noise in a viewer.
        if self.event_type.as_deref() == Some("workingLocation") {
            return None;
        }
        let when = parse_when(self.start.as_ref()?, self.end.as_ref())?;
        let reminders = match &self.reminders {
            // Google applies calendar defaults to timed events only.
            Some(r) if !r.use_default => popup_minutes(&r.overrides),
            _ if matches!(when, When::Timed { .. }) => calendar.default_reminders.clone(),
            _ => vec![],
        };
        let meet_link = self.hangout_link.clone().or_else(|| {
            self.conference_data.as_ref().and_then(|c| {
                c.entry_points
                    .iter()
                    .find(|e| e.entry_point_type.as_deref() == Some("video"))
                    .and_then(|e| e.uri.clone())
            })
        });
        let attendees = self
            .attendees
            .into_iter()
            .map(|a| Attendee {
                name: a
                    .display_name
                    .clone()
                    .or_else(|| a.email.clone())
                    .unwrap_or_default(),
                email: a.email.unwrap_or_default(),
                response: match a.response_status.as_deref() {
                    Some("accepted") => Response::Accepted,
                    Some("declined") => Response::Declined,
                    Some("tentative") => Response::Tentative,
                    _ => Response::NeedsAction,
                },
                organizer: a.organizer,
                is_self: a.is_self,
            })
            .collect();
        Some(Event {
            id: self.id,
            calendar_id: calendar.id.clone(),
            title: self
                .summary
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| "(No title)".into()),
            when,
            color: self.color_id.as_deref().and_then(google_event_color),
            location: self.location.filter(|s| !s.trim().is_empty()),
            description: self
                .description
                .map(|d| html_to_text(&d))
                .filter(|s| !s.is_empty()),
            html_link: self.html_link,
            meet_link,
            organizer: self.organizer.and_then(|o| o.display_name.or(o.email)),
            attendees,
            reminders,
            tentative: self.status.as_deref() == Some("tentative"),
        })
    }
}

fn parse_when(start: &WireTime, end: Option<&WireTime>) -> Option<When> {
    if let Some(d) = &start.date {
        let start: jiff::civil::Date = d.parse().ok()?;
        let end = end
            .and_then(|e| e.date.as_ref())
            .and_then(|d| d.parse().ok())
            .filter(|e| *e > start)
            .unwrap_or_else(|| start.tomorrow().unwrap_or(start));
        return Some(When::AllDay { start, end });
    }
    let start: Timestamp = start.date_time.as_ref()?.parse().ok()?;
    let end = end
        .and_then(|e| e.date_time.as_ref())
        .and_then(|d| d.parse().ok())
        .filter(|e: &Timestamp| *e >= start)
        .unwrap_or(start);
    Some(When::Timed { start, end })
}

/// Google stores descriptions as light HTML. Keep line structure and text, drop markup.
pub fn html_to_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        out.push_str(&rest[..lt]);
        let Some(gt) = rest[lt..].find('>') else {
            out.push_str(&rest[lt..]);
            rest = "";
            break;
        };
        let tag = rest[lt + 1..lt + gt].trim().to_ascii_lowercase();
        let name = tag
            .trim_start_matches('/')
            .split(|c: char| c.is_whitespace() || c == '/')
            .next()
            .unwrap_or_default();
        match name {
            "br" => out.push('\n'),
            "p" | "div" | "li" | "ul" | "ol" | "tr" | "h1" | "h2" | "h3"
                if tag.starts_with('/') =>
            {
                out.push('\n')
            }
            "li" => out.push_str("• "),
            _ => {}
        }
        rest = &rest[lt + gt + 1..];
    }
    out.push_str(rest);
    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&");
    let mut lines: Vec<&str> = decoded.lines().map(str::trim_end).collect();
    // Collapse runs of blank lines and trim the ends.
    lines.dedup_by(|a, b| a.is_empty() && b.is_empty());
    lines.join("\n").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn cal() -> Calendar {
        Calendar {
            id: Calendar::key("me@example.com", "me@example.com"),
            account: "me@example.com".into(),
            remote_id: "me@example.com".into(),
            name: "Me".into(),
            color: 0x123456,
            primary: true,
            default_reminders: vec![10],
            selected_upstream: true,
        }
    }

    fn parse(json: &str) -> Vec<Event> {
        let r: EventsResponse = serde_json::from_str(json).unwrap();
        r.items
            .into_iter()
            .filter_map(|e| e.into_model(&cal()))
            .collect()
    }

    #[test]
    fn timed_event_with_default_reminders_and_meet() {
        let evs = parse(
            r#"{"items":[{"id":"a","status":"confirmed","summary":"Standup",
            "start":{"dateTime":"2026-10-21T09:30:00+01:00","timeZone":"Europe/London"},
            "end":{"dateTime":"2026-10-21T09:45:00+01:00"},
            "reminders":{"useDefault":true},
            "conferenceData":{"entryPoints":[{"entryPointType":"phone","uri":"tel:1"},{"entryPointType":"video","uri":"https://meet.google.com/x"}]},
            "attendees":[{"email":"me@example.com","self":true,"responseStatus":"declined"},{"email":"b@x","displayName":"Bea","organizer":true,"responseStatus":"accepted"}],
            "colorId":"11"}]}"#,
        );
        assert_eq!(evs.len(), 1);
        let e = &evs[0];
        assert_eq!(e.title, "Standup");
        assert_eq!(
            e.when,
            When::Timed {
                start: "2026-10-21T08:30:00Z".parse().unwrap(),
                end: "2026-10-21T08:45:00Z".parse().unwrap()
            }
        );
        assert_eq!(e.reminders, vec![10]);
        assert_eq!(e.meet_link.as_deref(), Some("https://meet.google.com/x"));
        assert!(e.declined());
        assert_eq!(e.color, Some(0xd50000));
        assert_eq!(e.attendees[1].name, "Bea");
    }

    #[test]
    fn all_day_and_overrides_and_skips() {
        let evs = parse(
            r#"{"items":[
            {"id":"x","status":"cancelled"},
            {"id":"w","eventType":"workingLocation","start":{"date":"2026-10-21"},"end":{"date":"2026-10-22"}},
            {"id":"b","start":{"date":"2026-10-21"},"end":{"date":"2026-10-24"},"reminders":{"useDefault":true}},
            {"id":"c","summary":"  ","start":{"dateTime":"2026-10-21T10:00:00Z"},"end":{"dateTime":"2026-10-21T11:00:00Z"},
             "reminders":{"useDefault":false,"overrides":[{"method":"email","minutes":60},{"method":"popup","minutes":30},{"method":"popup","minutes":5}]}}
            ]}"#,
        );
        assert_eq!(evs.len(), 2);
        assert_eq!(
            evs[0].when,
            When::AllDay {
                start: date(2026, 10, 21),
                end: date(2026, 10, 24)
            }
        );
        assert!(evs[0].reminders.is_empty());
        assert_eq!(evs[1].title, "(No title)");
        assert_eq!(evs[1].reminders, vec![5, 30]);
        assert!(evs[1].self_response().is_none());
    }

    #[test]
    fn calendar_list_mapping() {
        let r: CalendarListResponse = serde_json::from_str(
            r##"{"items":[
            {"id":"h","hidden":true},
            {"id":"me@x","summary":"me@x","primary":true,"backgroundColor":"#9fe1e7",
             "defaultReminders":[{"method":"popup","minutes":10},{"method":"email","minutes":30}]},
            {"id":"team","summary":"Team","summaryOverride":"Our team","selected":false}
            ]}"##,
        )
        .unwrap();
        let cals: Vec<_> = r
            .items
            .into_iter()
            .filter_map(|c| c.into_model("me@x"))
            .collect();
        assert_eq!(cals.len(), 2);
        assert_eq!(cals[1].id, "me@x/team");
        assert_eq!(cals[1].remote_id, "team");
        assert_eq!(cals[1].account, "me@x");
        assert_eq!(cals[0].color, 0x9fe1e7);
        assert_eq!(cals[0].default_reminders, vec![10]);
        assert!(cals[0].selected_upstream);
        assert_eq!(cals[1].name, "Our team");
        assert!(!cals[1].selected_upstream);
    }

    #[test]
    fn html_flattening() {
        assert_eq!(
            html_to_text(
                "Agenda:<br>1. <b>Intro</b><br/><br><br>See <a href=\"x\">doc</a> &amp; notes&nbsp;"
            ),
            "Agenda:\n1. Intro\n\nSee doc & notes"
        );
        assert_eq!(html_to_text("<ul><li>a</li><li>b</li></ul>"), "• a\n• b");
        assert_eq!(html_to_text("plain 1 < 2"), "plain 1 < 2");
    }
}
