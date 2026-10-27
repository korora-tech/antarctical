//! Google Calendar API v3: calendar and event reads plus opt-in RSVP updates. `singleEvents=true`
//! expands recurrences so replies target only the selected occurrence.

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
const ATTENDEE_FIELDS: &str = "etag,attendeesOmitted,attendees(email,displayName,responseStatus,organizer,self,comment,additionalGuests,optional,resource)";

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
    InvalidRequest(String),
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
            ApiError::InvalidRequest(m) => write!(f, "Cannot update this invitation: {m}"),
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
        #[serde(default)]
        errors: Vec<Detail>,
    }
    #[derive(Deserialize)]
    struct Detail {
        reason: Option<String>,
    }
    serde_json::from_str::<Envelope>(body)
        .map(|e| {
            let reason = e.error.errors.into_iter().find_map(|detail| detail.reason);
            match reason {
                Some(reason) => format!("{} ({reason})", e.error.message),
                None => e.error.message,
            }
        })
        .unwrap_or_else(|_| body.chars().take(200).collect())
}

fn http_error(status: u16, body: &str) -> ApiError {
    match status {
        401 => ApiError::Unauthorized,
        404 | 410 => ApiError::NotFound,
        429 => ApiError::RateLimited,
        403 if body.contains("rateLimitExceeded") || body.contains("RATE_LIMIT") => {
            ApiError::RateLimited
        }
        403 => ApiError::Forbidden(error_message(body)),
        _ => ApiError::Http {
            status,
            body: error_message(body),
        },
    }
}

impl ApiError {
    pub fn missing_scope(&self) -> bool {
        matches!(
            self,
            ApiError::Forbidden(message)
                if {
                    let message = message.to_ascii_lowercase();
                    message.contains("insufficient authentication scopes")
                        || message.contains("insufficientpermissions")
                }
        )
    }
}

/// The account's email: the id of its primary calendar (available under `calendarList.readonly`).
pub fn primary_email(agent: &ureq::Agent, token: &str) -> Result<String, ApiError> {
    let mut page: Option<String> = None;
    loop {
        let mut url = url::Url::parse(&format!("{BASE}/users/me/calendarList"))
            .expect("BASE and the calendar-list path form a valid Google API URL");
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("maxResults", "250")
                .append_pair("fields", "items(id,primary),nextPageToken");
            if let Some(page) = &page {
                query.append_pair("pageToken", page);
            }
        }
        let result: PrimaryCalendarList = get(agent, &url, token)?;
        if let Some(email) = primary_calendar_id(result.items) {
            return Ok(email);
        }
        let Some(next) = result.next_page_token else {
            break;
        };
        page = Some(next);
    }
    Err(ApiError::Decode(
        "Google did not return a primary calendar in its calendar list".into(),
    ))
}

fn primary_calendar_id(items: Vec<PrimaryCalendar>) -> Option<String> {
    items
        .into_iter()
        .find(|calendar| calendar.primary)
        .map(|calendar| calendar.id)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PrimaryCalendarList {
    #[serde(default)]
    items: Vec<PrimaryCalendar>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct PrimaryCalendar {
    id: String,
    #[serde(default)]
    primary: bool,
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

#[cfg(test)]
fn response_patch_body(event: &Event, response: Response) -> Result<serde_json::Value, ApiError> {
    response_patch_body_from_attendees(&event.attendees, response)
}

fn response_patch_body_from_attendees(
    attendees: &[Attendee],
    response: Response,
) -> Result<serde_json::Value, ApiError> {
    let self_attendees = attendees.iter().filter(|a| a.is_self).count();
    if self_attendees != 1 || attendees.iter().any(|a| a.is_self && a.organizer) {
        return Err(ApiError::InvalidRequest(
            "the event must have exactly one non-organizer self attendee".into(),
        ));
    }
    if attendees.iter().any(|a| a.email.is_empty()) {
        return Err(ApiError::InvalidRequest(
            "an attendee has no email address to preserve".into(),
        ));
    }

    let attendees: Vec<serde_json::Value> = attendees
        .iter()
        .map(|attendee| {
            let mut fields = serde_json::Map::new();
            fields.insert(
                "email".into(),
                serde_json::Value::String(attendee.email.clone()),
            );
            if let Some(name) = &attendee.display_name {
                fields.insert(
                    "displayName".into(),
                    serde_json::Value::String(name.clone()),
                );
            }
            if let Some(comment) = &attendee.comment {
                fields.insert("comment".into(), serde_json::Value::String(comment.clone()));
            }
            if let Some(guests) = attendee.additional_guests {
                fields.insert("additionalGuests".into(), serde_json::json!(guests));
            }
            if let Some(optional) = attendee.optional {
                fields.insert("optional".into(), serde_json::json!(optional));
            }
            if let Some(resource) = attendee.resource {
                fields.insert("resource".into(), serde_json::json!(resource));
            }
            let status = if attendee.is_self {
                response
            } else {
                attendee.response
            };
            fields.insert(
                "responseStatus".into(),
                serde_json::Value::String(google_response(status).into()),
            );
            serde_json::Value::Object(fields)
        })
        .collect();
    Ok(serde_json::json!({"attendees": attendees}))
}

fn response_patch_body_from_self_attendee(
    attendees: &[Attendee],
    response: Response,
) -> Result<serde_json::Value, ApiError> {
    let mut self_attendees = attendees.iter().filter(|attendee| attendee.is_self);
    let Some(attendee) = self_attendees.next() else {
        return Err(ApiError::InvalidRequest(
            "the event must have exactly one non-organizer self attendee".into(),
        ));
    };
    if attendee.organizer || self_attendees.next().is_some() {
        return Err(ApiError::InvalidRequest(
            "the event must have exactly one non-organizer self attendee".into(),
        ));
    }
    if attendee.email.is_empty() {
        return Err(ApiError::InvalidRequest(
            "the self attendee has no email address to preserve".into(),
        ));
    }
    Ok(serde_json::json!({
        "attendeesOmitted": true,
        "attendees": [{
            "email": attendee.email,
            "responseStatus": google_response(response)
        }]
    }))
}

fn google_response(response: Response) -> &'static str {
    match response {
        Response::Accepted => "accepted",
        Response::Declined => "declined",
        Response::Tentative => "tentative",
        Response::NeedsAction => "needsAction",
    }
}

/// Updates only this occurrence's RSVP from a fresh ETagged attendee snapshot.
pub fn patch_response(
    agent: &ureq::Agent,
    token: &str,
    calendar: &Calendar,
    event: &Event,
    response: Response,
) -> Result<(), ApiError> {
    patch_response_at(agent, token, calendar, event, response, BASE)
}

fn patch_response_at(
    agent: &ureq::Agent,
    token: &str,
    calendar: &Calendar,
    event: &Event,
    response: Response,
    api_base: &str,
) -> Result<(), ApiError> {
    let mut get_url = event_url(api_base, &calendar.remote_id, &event.id);
    get_url
        .query_pairs_mut()
        .append_pair("fields", ATTENDEE_FIELDS);
    let current: EventAttendees = get(agent, &get_url, token)?;
    let (etag, body) = patch_body_from_current(current, response)?;

    let url = event_url(api_base, &calendar.remote_id, &event.id);
    let request = agent
        .patch(url.as_str())
        .header("Authorization", &format!("Bearer {token}"))
        .header("If-Match", &etag);
    let mut result = request
        .send_json(&body)
        .map_err(|e| ApiError::Network(e.to_string()))?;
    let status = result.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(());
    }
    let body = result
        .body_mut()
        .with_config()
        .limit(BODY_LIMIT)
        .read_to_string()
        .map_err(|e| ApiError::Network(e.to_string()))?;
    Err(http_error(status, &body))
}

fn event_url(api_base: &str, calendar_id: &str, event_id: &str) -> url::Url {
    let mut url = url::Url::parse(api_base).expect("the Calendar API base is a valid URL");
    url.path_segments_mut()
        .expect("the API base has no query or fragment, so its path segments are mutable")
        .push("calendars")
        .push(calendar_id)
        .push("events")
        .push(event_id);
    url
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
struct EventAttendees {
    etag: Option<String>,
    #[serde(default)]
    attendees_omitted: bool,
    #[serde(default)]
    attendees: Vec<WireAttendee>,
}

fn patch_body_from_current(
    current: EventAttendees,
    response: Response,
) -> Result<(String, serde_json::Value), ApiError> {
    let attendees_omitted = current.attendees_omitted;
    let etag = current.etag.ok_or_else(|| {
        ApiError::InvalidRequest(
            "Google did not provide an event version; no response was changed".into(),
        )
    })?;
    let attendees: Vec<Attendee> = current
        .attendees
        .into_iter()
        .map(WireAttendee::into_model)
        .collect();
    let body = if attendees_omitted {
        response_patch_body_from_self_attendee(&attendees, response)?
    } else {
        response_patch_body_from_attendees(&attendees, response)?
    };
    Ok((etag, body))
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
    comment: Option<String>,
    additional_guests: Option<u32>,
    optional: Option<bool>,
    resource: Option<bool>,
}

impl WireAttendee {
    fn into_model(self) -> Attendee {
        Attendee {
            name: self
                .display_name
                .clone()
                .or_else(|| self.email.clone())
                .unwrap_or_default(),
            email: self.email.unwrap_or_default(),
            display_name: self.display_name,
            comment: self.comment,
            additional_guests: self.additional_guests,
            optional: self.optional,
            resource: self.resource,
            response: match self.response_status.as_deref() {
                Some("accepted") => Response::Accepted,
                Some("declined") => Response::Declined,
                Some("tentative") => Response::Tentative,
                _ => Response::NeedsAction,
            },
            organizer: self.organizer,
            is_self: self.is_self,
        }
    }
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
            .map(WireAttendee::into_model)
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
    fn regular_event_list_does_not_fetch_on_demand_attendee_fields() {
        for field in ["comment", "additionalGuests", "optional", "resource"] {
            assert!(
                !EVENT_FIELDS.contains(field),
                "regular sync requested {field}"
            );
            assert!(
                ATTENDEE_FIELDS.contains(field),
                "reply fetch omitted {field}"
            );
        }
    }

    #[test]
    fn forbidden_scope_errors_are_distinguished_from_other_forbidden_errors() {
        let missing_scope = http_error(
            403,
            r#"{"error":{"message":"Permission denied","errors":[{"reason":"insufficientPermissions"}]}}"#,
        );
        assert!(missing_scope.missing_scope());
        assert!(!ApiError::Forbidden("calendar is not shared".into()).missing_scope());
    }

    #[test]
    fn response_patch_changes_only_the_self_attendee() {
        let events = parse(
            r#"{"items":[{"id":"occurrence-42",
            "start":{"dateTime":"2026-10-21T09:30:00Z"},
            "end":{"dateTime":"2026-10-21T10:00:00Z"},
            "attendees":[
              {"email":"me@example.com","self":true,"responseStatus":"needsAction"},
              {"email":"organizer@example.com","displayName":"Organizer","organizer":true,"responseStatus":"accepted"}
            ]}]}"#,
        );
        let body = response_patch_body(&events[0], Response::Tentative).unwrap();
        assert_eq!(body.as_object().unwrap().len(), 1);
        assert_eq!(
            body["attendees"],
            serde_json::json!([
                {"email":"me@example.com","responseStatus":"tentative"},
                {"email":"organizer@example.com","displayName":"Organizer","responseStatus":"accepted"}
            ])
        );
    }

    #[test]
    fn response_patch_preserves_writable_fields_for_every_attendee() {
        let events = parse(
            r#"{"items":[{"id":"occurrence-43",
            "start":{"dateTime":"2026-10-21T09:30:00Z"},
            "end":{"dateTime":"2026-10-21T10:00:00Z"},
            "attendees":[
              {"email":"me@example.com","displayName":"Me","self":true,"responseStatus":"accepted","comment":"Arriving late","additionalGuests":2,"optional":true,"resource":true},
              {"email":"guest@example.com","displayName":"Guest","responseStatus":"tentative","comment":"Needs access","optional":false,"resource":false}
            ]}]}"#,
        );
        let body = response_patch_body(&events[0], Response::Declined).unwrap();
        assert_eq!(
            body["attendees"],
            serde_json::json!([
                {"email":"me@example.com","displayName":"Me","responseStatus":"declined","comment":"Arriving late","additionalGuests":2,"optional":true,"resource":true},
                {"email":"guest@example.com","displayName":"Guest","responseStatus":"tentative","comment":"Needs access","optional":false,"resource":false}
            ])
        );
    }

    #[test]
    fn reply_patch_uses_attendees_omitted_to_update_only_the_self_response() {
        let truncated: EventAttendees = serde_json::from_str(
            r#"{"etag":"tag-1","attendeesOmitted":true,"attendees":[{"email":"me@example.com","self":true}]}"#,
        )
        .unwrap();
        let (etag, body) = patch_body_from_current(truncated, Response::Accepted).unwrap();
        assert_eq!(etag, "tag-1");
        assert_eq!(
            body,
            serde_json::json!({
                "attendeesOmitted": true,
                "attendees": [{
                    "email": "me@example.com",
                    "responseStatus": "accepted"
                }]
            })
        );
    }

    #[test]
    fn reply_patch_requires_an_etag_for_concurrent_update_protection() {
        let no_etag: EventAttendees =
            serde_json::from_str(r#"{"attendees":[{"email":"me@example.com","self":true}]}"#)
                .unwrap();
        assert!(matches!(
            patch_body_from_current(no_etag, Response::Accepted),
            Err(ApiError::InvalidRequest(_))
        ));
    }

    #[test]
    fn patch_request_uses_fresh_etag_and_maps_precondition_conflict() {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};
        use std::thread;
        use std::time::{Duration, Instant};

        fn accept_with_timeout(listener: &TcpListener) -> TcpStream {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                match listener.accept() {
                    Ok((stream, _)) => {
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        return stream;
                    }
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(error) => panic!("mock Calendar API did not receive a request: {error}"),
                }
            }
        }

        fn receive_request(stream: &mut TcpStream) -> String {
            let mut request = Vec::new();
            loop {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let headers = String::from_utf8_lossy(&request);
            let body_len = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            let mut body = vec![0; body_len];
            stream.read_exact(&mut body).unwrap();
            request.extend(body);
            String::from_utf8(request).unwrap()
        }

        fn respond(stream: &mut TcpStream, status: &str, body: &str) {
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            stream.flush().unwrap();
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = thread::spawn(move || {
            let mut get_stream = accept_with_timeout(&listener);
            let get_request = receive_request(&mut get_stream);
            respond(
                &mut get_stream,
                "200 OK",
                r#"{"etag":"etag-7","attendees":[{"email":"me@example.com","self":true,"responseStatus":"needsAction"},{"email":"organizer@example.com","organizer":true,"responseStatus":"accepted"}]}"#,
            );

            let mut patch_stream = accept_with_timeout(&listener);
            let patch_request = receive_request(&mut patch_stream);
            respond(
                &mut patch_stream,
                "412 Precondition Failed",
                r#"{"error":{"message":"event changed"}}"#,
            );
            (get_request, patch_request)
        });

        let event = parse(
            r#"{"items":[{"id":"occurrence-42","start":{"dateTime":"2026-10-21T09:30:00Z"},"end":{"dateTime":"2026-10-21T10:00:00Z"},"attendees":[{"email":"me@example.com","self":true,"responseStatus":"needsAction"}]}]}"#,
        )
        .remove(0);
        let api_base = format!("http://{address}/calendar/v3");
        let result = patch_response_at(
            &agent(),
            "access-token",
            &cal(),
            &event,
            Response::Declined,
            &api_base,
        );
        let (get_request, patch_request) = server.join().unwrap();

        assert!(matches!(
            result,
            Err(ApiError::Http { status: 412, ref body }) if body == "event changed"
        ));
        assert!(get_request.starts_with("GET /calendar/v3/calendars/"));
        assert!(
            get_request
                .lines()
                .next()
                .unwrap()
                .contains("/events/occurrence-42?fields=")
        );
        assert!(patch_request.starts_with("PATCH /calendar/v3/calendars/"));
        assert!(
            patch_request
                .lines()
                .next()
                .unwrap()
                .contains("/events/occurrence-42 HTTP/")
        );
        assert!(
            patch_request
                .to_ascii_lowercase()
                .contains("if-match: etag-7")
        );
        assert!(
            patch_request
                .to_ascii_lowercase()
                .contains("authorization: bearer access-token")
        );
        let body = patch_request.split_once("\r\n\r\n").unwrap().1;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            serde_json::json!({
                "attendees": [
                    {"email":"me@example.com","responseStatus":"declined"},
                    {"email":"organizer@example.com","responseStatus":"accepted"}
                ]
            })
        );
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
    fn primary_calendar_id_comes_from_calendar_list_entry() {
        let items: Vec<PrimaryCalendar> = serde_json::from_str(
            r#"[{"id":"work@example.com","primary":false},{"id":"me@example.com","primary":true}]"#,
        )
        .unwrap();
        assert_eq!(
            primary_calendar_id(items).as_deref(),
            Some("me@example.com")
        );
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
