//! Google Calendar API v3: calendar and event reads plus opt-in RSVP updates. `singleEvents=true`
//! expands recurrences so replies target only the selected occurrence.

use std::fmt;
use std::time::Duration;

use jiff::Timestamp;
use serde::Deserialize;

use crate::model::{
    Attendee, Calendar, Event, EventKind, Response, When, google_event_color, parse_hex_color,
};

const BASE: &str = "https://www.googleapis.com/calendar/v3";
const BODY_LIMIT: u64 = 32 * 1024 * 1024;

const CALENDAR_FIELDS: &str = concat!(
    "items(id,summary,summaryOverride,backgroundColor,primary,selected,hidden,defaultReminders,accessRole),",
    "nextPageToken",
);
const EVENT_FIELDS: &str = "items(id,status,summary,description,location,htmlLink,hangoutLink,\
    start,end,colorId,eventType,attendees(email,displayName,responseStatus,organizer,self),\
    organizer(email,displayName,self),reminders,conferenceData(entryPoints(entryPointType,uri)),\
    workingLocationProperties(type,officeLocation(label),customLocation(label))),\
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
            ApiError::InvalidRequest(m) => write!(f, "Cannot complete this change: {m}"),
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
            let status = if attendee.is_self {
                response
            } else {
                attendee.response
            };
            attendee_json(attendee, status)
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

// ----- event writes -------------------------------------------------------------------------

/// A single event's fields as entered by the user, for create and edit.
#[derive(Clone, Debug, PartialEq)]
pub struct EventDraft {
    pub title: String,
    pub when: When,
    /// IANA zone name (e.g. `Europe/London`). Required by Google for recurring events.
    pub time_zone: Option<String>,
    pub location: Option<String>,
    pub description: Option<String>,
    /// Guest email addresses. Applied on create only; editing never rewrites the attendee list,
    /// so existing RSVPs are preserved.
    pub guests: Vec<String>,
    /// Ask Google to attach a Meet link (create only).
    pub meet: bool,
    /// An `RRULE` body such as `FREQ=WEEKLY;BYDAY=MO` (create only).
    pub rrule: Option<String>,
    /// Edit only: the guests the event should have, besides the organiser, you and any rooms.
    /// `None` leaves the guest list alone. Existing guests keep their RSVPs.
    pub edit_guests: Option<Vec<String>>,
    /// Edit only: `None` leaves the reminders alone.
    pub edit_reminders: Option<RemindersEdit>,
    /// Edit only: apply the edit to the whole repeating series instead of one occurrence.
    pub series: Option<SeriesEdit>,
}

/// How an edit of a whole repeating series treats the time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeriesEdit {
    /// Move every occurrence to the time of day (and length) in the draft. Their dates stay.
    pub change_time: bool,
    /// Only this occurrence and the ones after it: the series is cut before it and a new series
    /// carries the edit from there on.
    pub following: bool,
}

/// The id of the series an occurrence belongs to (`<series>_<date>`), if `id` is an occurrence.
pub fn series_id(instance_id: &str) -> Option<&str> {
    instance_id.rsplit_once('_').map(|(series, _)| series)
}

/// A change to an event's reminders.
#[derive(Clone, Debug, PartialEq)]
pub enum RemindersEdit {
    /// Use the calendar's default reminders.
    Defaults,
    /// Exactly these pop-up reminders, in minutes before the start (empty: none).
    Custom(Vec<i64>),
}

const EVENT_FIELDS_ONE: &str = "id,status,summary,description,location,htmlLink,hangoutLink,\
    start,end,colorId,eventType,attendees(email,displayName,responseStatus,organizer,self),\
    organizer(email,displayName,self),reminders,conferenceData(entryPoints(entryPointType,uri)),\
    workingLocationProperties(type,officeLocation(label),customLocation(label))";

fn wire_when(when: &When, tz: Option<&str>) -> (serde_json::Value, serde_json::Value) {
    use serde_json::json;
    match when {
        When::AllDay { start, end } => (
            json!({ "date": start.to_string() }),
            json!({ "date": end.to_string() }),
        ),
        When::Timed { start, end } => {
            let wrap = |t: &Timestamp| {
                let mut v = json!({ "dateTime": t.to_string() });
                if let Some(tz) = tz {
                    v["timeZone"] = json!(tz);
                }
                v
            };
            (wrap(start), wrap(end))
        }
    }
}

/// The JSON body for `events.insert`. `request_id` makes Meet creation idempotent.
fn insert_body(draft: &EventDraft, request_id: &str) -> serde_json::Value {
    use serde_json::json;
    let (start, end) = wire_when(&draft.when, draft.time_zone.as_deref());
    let mut body = json!({
        "summary": draft.title,
        "start": start,
        "end": end,
    });
    if let Some(l) = draft.location.as_deref().filter(|l| !l.is_empty()) {
        body["location"] = json!(l);
    }
    if let Some(d) = draft.description.as_deref().filter(|d| !d.is_empty()) {
        body["description"] = json!(d);
    }
    if !draft.guests.is_empty() {
        body["attendees"] = draft.guests.iter().map(|e| json!({ "email": e })).collect();
    }
    if let Some(r) = &draft.rrule {
        body["recurrence"] = json!([format!("RRULE:{r}")]);
    }
    if draft.meet {
        body["conferenceData"] = json!({
            "createRequest": {
                "requestId": request_id,
                "conferenceSolutionKey": { "type": "hangoutsMeet" },
            }
        });
    }
    body
}

/// The JSON body for `events.patch`: only fields an edit can change. Title, time and location
/// are always sent (an empty location clears it). The description is sent only when `Some`
/// (`Some("")` clears it), because the loaded copy is flattened text and writing it back would
/// destroy Google's formatting. Attendees, recurrence and conference data are never sent.
fn patch_body(draft: &EventDraft) -> serde_json::Value {
    use serde_json::json;
    let (start, end) = wire_when(&draft.when, draft.time_zone.as_deref());
    let nullable = |v: &str| {
        if v.is_empty() {
            serde_json::Value::Null
        } else {
            json!(v)
        }
    };
    let mut body = json!({
        "summary": draft.title,
        "start": start,
        "end": end,
        "location": nullable(draft.location.as_deref().unwrap_or("")),
    });
    if let Some(d) = draft.description.as_deref() {
        body["description"] = nullable(d);
    }
    match &draft.edit_reminders {
        None => {}
        Some(RemindersEdit::Defaults) => body["reminders"] = json!({ "useDefault": true }),
        Some(RemindersEdit::Custom(minutes)) => {
            body["reminders"] = json!({
                "useDefault": false,
                "overrides": minutes
                    .iter()
                    .map(|m| json!({ "method": "popup", "minutes": m }))
                    .collect::<Vec<_>>(),
            });
        }
    }
    body
}

/// One attendee as sent back to Google, keeping everything it told us about them.
fn attendee_json(attendee: &Attendee, status: Response) -> serde_json::Value {
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
    fields.insert(
        "responseStatus".into(),
        serde_json::Value::String(google_response(status).into()),
    );
    serde_json::Value::Object(fields)
}

/// The attendee array for an edit that makes `wanted` the guest list. The organiser, you and
/// rooms always stay; everyone already invited keeps their RSVP; new addresses are added with no
/// response. `None` when nothing would change.
fn merged_attendees(
    current: &[Attendee],
    wanted: &[String],
) -> Result<Option<Vec<serde_json::Value>>, ApiError> {
    if current.iter().any(|a| a.email.is_empty()) {
        return Err(ApiError::InvalidRequest(
            "a guest has no email address to preserve".into(),
        ));
    }
    let wanted: Vec<String> = wanted.iter().map(|e| e.trim().to_lowercase()).collect();
    let keeps = |a: &Attendee| {
        a.organizer
            || a.is_self
            || a.resource == Some(true)
            || wanted.contains(&a.email.to_lowercase())
    };
    let mut out: Vec<serde_json::Value> = current
        .iter()
        .filter(|a| keeps(a))
        .map(|a| attendee_json(a, a.response))
        .collect();
    let removed = out.len() != current.len();
    let mut added = false;
    for email in &wanted {
        let known = current.iter().any(|a| a.email.to_lowercase() == *email)
            || out.iter().any(|v| {
                v["email"]
                    .as_str()
                    .is_some_and(|e| e.to_lowercase() == *email)
            });
        if !known && !email.is_empty() {
            out.push(serde_json::json!({ "email": email }));
            added = true;
        }
    }
    Ok((removed || added).then_some(out))
}

fn random_request_id() -> String {
    let mut buf = [0u8; 12];
    // A fixed id is still valid, only not unique; Google just uses it to deduplicate retries.
    if getrandom::fill(&mut buf).is_err() {
        buf = [7; 12];
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// Sends a JSON request and returns the response body of a 2xx, or the mapped error.
fn send_write(
    agent: &ureq::Agent,
    method: &str,
    url: &url::Url,
    token: &str,
    body: Option<&serde_json::Value>,
    if_match: Option<&str>,
) -> Result<String, ApiError> {
    let auth = format!("Bearer {token}");
    let mut resp = match (method, body) {
        ("POST", Some(b)) => agent
            .post(url.as_str())
            .header("Authorization", &auth)
            .send_json(b),
        ("POST", None) => agent
            .post(url.as_str())
            .header("Authorization", &auth)
            .send_empty(),
        ("PATCH", Some(b)) => {
            let request = agent.patch(url.as_str()).header("Authorization", &auth);
            match if_match {
                Some(etag) => request.header("If-Match", etag).send_json(b),
                None => request.send_json(b),
            }
        }
        ("DELETE", None) => agent
            .delete(url.as_str())
            .header("Authorization", &auth)
            .call(),
        _ => return Err(ApiError::InvalidRequest("unsupported write".into())),
    }
    .map_err(|e| ApiError::Network(e.to_string()))?;
    let status = resp.status().as_u16();
    let text = resp
        .body_mut()
        .with_config()
        .limit(BODY_LIMIT)
        .read_to_string()
        .map_err(|e| ApiError::Network(e.to_string()))?;
    if (200..300).contains(&status) {
        Ok(text)
    } else {
        Err(http_error(status, &text))
    }
}

fn parse_written(body: &str, calendar: &Calendar) -> Result<Event, ApiError> {
    let wire: WireEvent =
        serde_json::from_str(body).map_err(|e| ApiError::Decode(e.to_string()))?;
    wire.into_model(calendar)
        .ok_or_else(|| ApiError::Decode("Google returned an event the app cannot display".into()))
}

/// Creates an event and returns it as Google stored it.
pub fn create_event(
    agent: &ureq::Agent,
    token: &str,
    calendar: &Calendar,
    draft: &EventDraft,
) -> Result<Event, ApiError> {
    create_event_at(agent, token, calendar, draft, BASE)
}

fn create_event_at(
    agent: &ureq::Agent,
    token: &str,
    calendar: &Calendar,
    draft: &EventDraft,
    api_base: &str,
) -> Result<Event, ApiError> {
    let mut url = url::Url::parse(api_base).expect("the Calendar API base is a valid URL");
    url.path_segments_mut()
        .expect("the API base has no query or fragment, so its path segments are mutable")
        .push("calendars")
        .push(&calendar.remote_id)
        .push("events");
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("fields", EVENT_FIELDS_ONE);
        if draft.meet {
            q.append_pair("conferenceDataVersion", "1");
        }
        // Inviting people emails them; only do it when the user typed guests.
        if !draft.guests.is_empty() {
            q.append_pair("sendUpdates", "all");
        }
    }
    let body = insert_body(draft, &random_request_id());
    let text = send_write(agent, "POST", &url, token, Some(&body), None)?;
    parse_written(&text, calendar)
}

/// Edits one event (or, for a recurring series, one occurrence) and returns the result.
pub fn update_event(
    agent: &ureq::Agent,
    token: &str,
    calendar: &Calendar,
    event_id: &str,
    draft: &EventDraft,
) -> Result<Event, ApiError> {
    update_event_at(agent, token, calendar, event_id, draft, BASE)
}

fn update_event_at(
    agent: &ureq::Agent,
    token: &str,
    calendar: &Calendar,
    event_id: &str,
    draft: &EventDraft,
    api_base: &str,
) -> Result<Event, ApiError> {
    let body = patch_body(draft);
    patch_with_guests(agent, token, calendar, event_id, body, draft, api_base)
}

/// Edits every occurrence of the series that `instance_id` belongs to. The title, location,
/// guests and reminders go to the series itself; with `change_time` the series' first
/// occurrence moves to the draft's time of day and length, on its own date, which shifts the
/// whole series.
fn update_series_at(
    agent: &ureq::Agent,
    token: &str,
    calendar: &Calendar,
    instance_id: &str,
    draft: &EventDraft,
    tz: &jiff::tz::TimeZone,
    api_base: &str,
) -> Result<(), ApiError> {
    if draft.series.is_some_and(|s| s.following) {
        return update_following_at(agent, token, calendar, instance_id, draft, api_base);
    }
    let series = series_id(instance_id)
        .ok_or_else(|| ApiError::InvalidRequest("this is not part of a repeating series".into()))?;
    let mut body = patch_body(draft);
    let object = body.as_object_mut().expect("a patch body is a JSON object");
    object.remove("start");
    object.remove("end");
    if draft.series.is_some_and(|s| s.change_time) {
        let When::Timed {
            start: new_start,
            end: new_end,
        } = draft.when
        else {
            return Err(ApiError::InvalidRequest(
                "only the time of timed events can be changed for a series".into(),
            ));
        };
        let mut url = event_url(api_base, &calendar.remote_id, series);
        url.query_pairs_mut().append_pair("fields", "start,end");
        let master: SeriesTimes = get(agent, &url, token)?;
        let first = master
            .start
            .and_then(|t| t.date_time)
            .and_then(|t| t.parse::<Timestamp>().ok())
            .ok_or_else(|| {
                ApiError::InvalidRequest("this series has no start time to change".into())
            })?;
        let time_of_day = new_start.to_zoned(tz.clone()).time();
        let date = first.to_zoned(tz.clone()).date();
        let start = date
            .at(
                time_of_day.hour(),
                time_of_day.minute(),
                time_of_day.second(),
                0,
            )
            .to_zoned(tz.clone())
            .map_err(|e| ApiError::InvalidRequest(e.to_string()))?
            .timestamp();
        let end = start
            + new_end
                .duration_since(new_start)
                .max(jiff::SignedDuration::ZERO);
        let (s, e) = wire_when(&When::Timed { start, end }, draft.time_zone.as_deref());
        object.insert("start".into(), s);
        object.insert("end".into(), e);
    }
    patch_with_guests(agent, token, calendar, series, body, draft, api_base).map(|_| ())
}

/// Where an occurrence of a series starts, read from its id (`<series>_<date>` for all-day
/// events, `<series>_<yyyymmddThhmmssZ>` for timed ones).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InstanceStart {
    Date(jiff::civil::Date),
    Instant(Timestamp),
}

fn instance_start(instance_id: &str) -> Option<InstanceStart> {
    let (_, suffix) = instance_id.rsplit_once('_')?;
    if let Ok(date) = jiff::civil::Date::strptime("%Y%m%d", suffix) {
        return Some(InstanceStart::Date(date));
    }
    let at = jiff::civil::DateTime::strptime("%Y%m%dT%H%M%SZ", suffix).ok()?;
    Some(InstanceStart::Instant(
        at.to_zoned(jiff::tz::TimeZone::UTC).ok()?.timestamp(),
    ))
}

/// The `UNTIL` value that ends a series just before the occurrence that starts at `start`.
fn until_before(start: InstanceStart) -> Option<String> {
    Some(match start {
        InstanceStart::Date(d) => d.yesterday().ok()?.strftime("%Y%m%d").to_string(),
        InstanceStart::Instant(t) => (t - jiff::SignedDuration::from_secs(1))
            .strftime("%Y%m%dT%H%M%SZ")
            .to_string(),
    })
}

/// The body of an `RRULE:` line without its `UNTIL` and `COUNT` parts.
fn rule_without_end(rule: &str) -> String {
    rule.split(';')
        .filter(|p| !p.starts_with("UNTIL=") && !p.starts_with("COUNT="))
        .collect::<Vec<_>>()
        .join(";")
}

#[derive(Deserialize)]
struct SeriesMaster {
    #[serde(default)]
    recurrence: Vec<String>,
    description: Option<String>,
    #[serde(default)]
    attendees: Vec<WireAttendee>,
    reminders: Option<serde_json::Value>,
}

/// The `recurrence` lines of a series, cut so that it ends before `until`. Errors when the
/// series has no plain rule or counts its repeats (a count cannot be split without listing them).
fn cut_recurrence(lines: &[String], until: &str) -> Result<(Vec<String>, String), ApiError> {
    let mut rule = None;
    let mut cut = Vec::new();
    for line in lines {
        match line.strip_prefix("RRULE:") {
            Some(body) => {
                if body.split(';').any(|p| p.starts_with("COUNT=")) {
                    return Err(ApiError::InvalidRequest(
                        "a series with a fixed number of repeats can't be split; \
                         change one event or the whole series"
                            .into(),
                    ));
                }
                rule = Some(rule_without_end(body));
                cut.push(format!("RRULE:{};UNTIL={until}", rule_without_end(body)));
            }
            None => cut.push(line.clone()),
        }
    }
    rule.map(|r| (cut, r))
        .ok_or_else(|| ApiError::InvalidRequest("this series has no repeat rule to split".into()))
}

/// Edits this occurrence and every later one: a new series starting at the occurrence carries the
/// edit (and the original repeat rule), then the old series is cut just before it. The new series
/// is created first so a failure never loses the later occurrences. Meet links are not carried.
fn update_following_at(
    agent: &ureq::Agent,
    token: &str,
    calendar: &Calendar,
    instance_id: &str,
    draft: &EventDraft,
    api_base: &str,
) -> Result<(), ApiError> {
    let series = series_id(instance_id)
        .ok_or_else(|| ApiError::InvalidRequest("this is not part of a repeating series".into()))?;
    let until = instance_start(instance_id)
        .and_then(until_before)
        .ok_or_else(|| ApiError::InvalidRequest("this occurrence has an unreadable date".into()))?;
    let mut url = event_url(api_base, &calendar.remote_id, series);
    url.query_pairs_mut().append_pair(
        "fields",
        "recurrence,description,reminders,attendees(email,self,organizer,resource)",
    );
    let master: SeriesMaster = get(agent, &url, token)?;
    let (cut, rule) = cut_recurrence(&master.recurrence, &until)?;

    let mut new = draft.clone();
    new.rrule = Some(rule);
    new.meet = false;
    if new.description.is_none() {
        new.description = master.description.clone();
    }
    new.guests = match &draft.edit_guests {
        Some(wanted) => wanted.clone(),
        None => master
            .attendees
            .iter()
            .filter(|a| !a.is_self && !a.organizer && a.resource != Some(true))
            .filter_map(|a| a.email.clone())
            .collect(),
    };
    let mut body = insert_body(&new, &random_request_id());
    match &draft.edit_reminders {
        Some(RemindersEdit::Defaults) => {
            body["reminders"] = serde_json::json!({ "useDefault": true })
        }
        Some(RemindersEdit::Custom(minutes)) => {
            body["reminders"] = serde_json::json!({
                "useDefault": false,
                "overrides": minutes
                    .iter()
                    .map(|m| serde_json::json!({ "method": "popup", "minutes": m }))
                    .collect::<Vec<_>>(),
            });
        }
        None => {
            if let Some(r) = master.reminders {
                body["reminders"] = r;
            }
        }
    }
    let mut create = url::Url::parse(api_base).expect("the Calendar API base is a valid URL");
    create
        .path_segments_mut()
        .expect("the API base has no query or fragment, so its path segments are mutable")
        .push("calendars")
        .push(&calendar.remote_id)
        .push("events");
    create.query_pairs_mut().append_pair("fields", "id");
    if !new.guests.is_empty() {
        create.query_pairs_mut().append_pair("sendUpdates", "all");
    }
    let created = send_write(agent, "POST", &create, token, Some(&body), None)?;
    if let Err(e) = truncate_series_at(agent, token, calendar, series, &cut, api_base) {
        // Better to lose the new series than to leave two overlapping ones.
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&created)
            && let Some(id) = v["id"].as_str()
        {
            let _ = delete_event_at(agent, token, calendar, id, api_base);
        }
        return Err(e);
    }
    Ok(())
}

fn truncate_series_at(
    agent: &ureq::Agent,
    token: &str,
    calendar: &Calendar,
    series: &str,
    recurrence: &[String],
    api_base: &str,
) -> Result<(), ApiError> {
    let url = event_url(api_base, &calendar.remote_id, series);
    let body = serde_json::json!({ "recurrence": recurrence });
    send_write(agent, "PATCH", &url, token, Some(&body), None).map(|_| ())
}

fn delete_following_at(
    agent: &ureq::Agent,
    token: &str,
    calendar: &Calendar,
    instance_id: &str,
    api_base: &str,
) -> Result<(), ApiError> {
    let series = series_id(instance_id)
        .ok_or_else(|| ApiError::InvalidRequest("this is not part of a repeating series".into()))?;
    let until = instance_start(instance_id)
        .and_then(until_before)
        .ok_or_else(|| ApiError::InvalidRequest("this occurrence has an unreadable date".into()))?;
    let mut url = event_url(api_base, &calendar.remote_id, series);
    url.query_pairs_mut().append_pair("fields", "recurrence");
    let master: SeriesMaster = get(agent, &url, token)?;
    let (cut, _) = cut_recurrence(&master.recurrence, &until)?;
    truncate_series_at(agent, token, calendar, series, &cut, api_base)
}

/// Deletes this occurrence and every later one by ending the series before it.
pub fn delete_following(
    agent: &ureq::Agent,
    token: &str,
    calendar: &Calendar,
    instance_id: &str,
) -> Result<(), ApiError> {
    delete_following_at(agent, token, calendar, instance_id, BASE)
}

/// Public entry point for [`update_series_at`], in the system time zone.
pub fn update_series(
    agent: &ureq::Agent,
    token: &str,
    calendar: &Calendar,
    instance_id: &str,
    draft: &EventDraft,
) -> Result<(), ApiError> {
    update_series_at(
        agent,
        token,
        calendar,
        instance_id,
        draft,
        &jiff::tz::TimeZone::system(),
        BASE,
    )
}

#[derive(Deserialize)]
struct SeriesTimes {
    start: Option<WireTime>,
}

fn patch_with_guests(
    agent: &ureq::Agent,
    token: &str,
    calendar: &Calendar,
    event_id: &str,
    mut body: serde_json::Value,
    draft: &EventDraft,
    api_base: &str,
) -> Result<Event, ApiError> {
    let mut etag = None;
    if let Some(wanted) = &draft.edit_guests {
        // Merge into what Google holds now, not what we loaded earlier, and refuse to overwrite
        // if someone changes the event in between.
        let mut get_url = event_url(api_base, &calendar.remote_id, event_id);
        get_url
            .query_pairs_mut()
            .append_pair("fields", ATTENDEE_FIELDS);
        let current: EventAttendees = get(agent, &get_url, token)?;
        if current.attendees_omitted {
            return Err(ApiError::InvalidRequest(
                "this event has too many guests to edit here; use Google Calendar".into(),
            ));
        }
        let attendees: Vec<Attendee> = current
            .attendees
            .into_iter()
            .map(WireAttendee::into_model)
            .collect();
        if let Some(merged) = merged_attendees(&attendees, wanted)? {
            body["attendees"] = serde_json::Value::Array(merged);
            etag = Some(current.etag.ok_or_else(|| {
                ApiError::InvalidRequest(
                    "Google did not provide an event version; guests were not changed".into(),
                )
            })?);
        }
    }
    let mut url = event_url(api_base, &calendar.remote_id, event_id);
    {
        let mut q = url.query_pairs_mut();
        q.append_pair("fields", EVENT_FIELDS_ONE);
        // New guests are emailed an invitation (and removed ones told), as in Google's own UI.
        if etag.is_some() {
            q.append_pair("sendUpdates", "all");
        }
    }
    let text = send_write(agent, "PATCH", &url, token, Some(&body), etag.as_deref())?;
    parse_written(&text, calendar)
}

/// Moves an event to another calendar of the same account, then applies `draft` (if any) to it
/// there. Returns the event as Google stores it afterwards.
pub fn move_event(
    agent: &ureq::Agent,
    token: &str,
    from: &Calendar,
    event_id: &str,
    to: &Calendar,
    draft: Option<&EventDraft>,
) -> Result<Event, ApiError> {
    move_event_at(agent, token, from, event_id, to, draft, BASE)
}

fn move_event_at(
    agent: &ureq::Agent,
    token: &str,
    from: &Calendar,
    event_id: &str,
    to: &Calendar,
    draft: Option<&EventDraft>,
    api_base: &str,
) -> Result<Event, ApiError> {
    if from.account != to.account {
        return Err(ApiError::InvalidRequest(
            "events can only move between calendars of the same account".into(),
        ));
    }
    let mut url = event_url(api_base, &from.remote_id, event_id);
    url.path_segments_mut()
        .expect("the API base has no query or fragment, so its path segments are mutable")
        .push("move");
    url.query_pairs_mut()
        .append_pair("destination", &to.remote_id)
        .append_pair("fields", EVENT_FIELDS_ONE);
    let text = send_write(agent, "POST", &url, token, None, None)?;
    let moved = parse_written(&text, to)?;
    let Some(draft) = draft else {
        return Ok(moved);
    };
    // The id is unchanged by a move, so the edit goes to the same id on the new calendar.
    update_event_at(agent, token, to, event_id, draft, api_base).map_err(|e| {
        ApiError::InvalidRequest(format!(
            "the event was moved, but the other changes were not saved: {e}"
        ))
    })
}

/// Deletes an event (one occurrence for a recurring instance id). Already gone counts as done.
pub fn delete_event(
    agent: &ureq::Agent,
    token: &str,
    calendar: &Calendar,
    event_id: &str,
) -> Result<(), ApiError> {
    delete_event_at(agent, token, calendar, event_id, BASE)
}

fn delete_event_at(
    agent: &ureq::Agent,
    token: &str,
    calendar: &Calendar,
    event_id: &str,
    api_base: &str,
) -> Result<(), ApiError> {
    let url = event_url(api_base, &calendar.remote_id, event_id);
    match send_write(agent, "DELETE", &url, token, None, None) {
        Ok(_) | Err(ApiError::NotFound) => Ok(()),
        Err(e) => Err(e),
    }
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
    /// `owner` and `writer` can change events; `reader` and `freeBusyReader` cannot.
    access_role: Option<String>,
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
            // An absent role is assumed writable; a refused write rolls back with Google's message.
            writable: self
                .access_role
                .as_deref()
                .is_none_or(|r| matches!(r, "owner" | "writer")),
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
    working_location_properties: Option<WireWorkingLocation>,
    #[serde(default)]
    attendees: Vec<WireAttendee>,
    organizer: Option<WirePerson>,
    reminders: Option<WireReminders>,
    conference_data: Option<WireConference>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireWorkingLocation {
    #[serde(rename = "type")]
    kind: Option<String>,
    office_location: Option<WireLabel>,
    custom_location: Option<WireLabel>,
}

#[derive(Deserialize)]
struct WireLabel {
    label: Option<String>,
}

impl WireWorkingLocation {
    /// A name for the place when Google gave the entry no title: "Home", the office label, or
    /// the custom label.
    fn name(&self) -> Option<String> {
        let label = |l: &Option<WireLabel>| {
            l.as_ref()
                .and_then(|l| l.label.clone())
                .filter(|l| !l.trim().is_empty())
        };
        match self.kind.as_deref() {
            Some("homeOffice") => Some("Home".into()),
            Some("officeLocation") => label(&self.office_location).or(Some("Office".into())),
            Some("customLocation") => label(&self.custom_location),
            _ => None,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireTime {
    date: Option<String>,
    date_time: Option<String>,
    time_zone: Option<String>,
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
        let kind = EventKind::from_google(self.event_type.as_deref());
        let time_zone = self.start.as_ref().and_then(|t| t.time_zone.clone());
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
                .or_else(|| {
                    self.working_location_properties
                        .as_ref()
                        .and_then(WireWorkingLocation::name)
                })
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
            kind,
            time_zone: time_zone.filter(|_| matches!(when, When::Timed { .. })),
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
            writable: true,
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
    fn the_zone_an_event_was_set_in_is_kept_for_timed_events_only() {
        let evs = parse(
            r#"{"items":[
            {"id":"t","start":{"dateTime":"2026-10-21T10:00:00+02:00","timeZone":"Europe/Berlin"},"end":{"dateTime":"2026-10-21T11:00:00+02:00"}},
            {"id":"d","start":{"date":"2026-10-21","timeZone":"Europe/Berlin"},"end":{"date":"2026-10-22"}},
            {"id":"n","start":{"dateTime":"2026-10-21T10:00:00Z"},"end":{"dateTime":"2026-10-21T11:00:00Z"}}
            ]}"#,
        );
        assert_eq!(evs[0].time_zone.as_deref(), Some("Europe/Berlin"));
        assert_eq!(evs[1].time_zone, None);
        assert_eq!(evs[2].time_zone, None);
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
        // The cancelled entry is skipped; the working location is kept, flagged as such.
        assert_eq!(evs.len(), 3);
        assert_eq!(evs[0].kind, EventKind::WorkingLocation);
        assert_eq!(
            evs[1].when,
            When::AllDay {
                start: date(2026, 10, 21),
                end: date(2026, 10, 24)
            }
        );
        assert!(evs[1].reminders.is_empty());
        assert_eq!(evs[2].title, "(No title)");
        assert_eq!(evs[2].reminders, vec![5, 30]);
        assert!(evs[2].self_response().is_none());
    }

    #[test]
    fn google_event_types_are_recognised_and_working_locations_named() {
        let evs = parse(
            r#"{"items":[
            {"id":"o","eventType":"outOfOffice","summary":"Out of office","start":{"dateTime":"2026-10-21T10:00:00Z"},"end":{"dateTime":"2026-10-21T11:00:00Z"}},
            {"id":"f","eventType":"focusTime","summary":"Focus time","start":{"dateTime":"2026-10-21T12:00:00Z"},"end":{"dateTime":"2026-10-21T13:00:00Z"}},
            {"id":"h","eventType":"workingLocation","workingLocationProperties":{"type":"homeOffice"},"start":{"date":"2026-10-21"},"end":{"date":"2026-10-22"}},
            {"id":"w","eventType":"workingLocation","workingLocationProperties":{"type":"officeLocation","officeLocation":{"label":"King's Cross"}},"start":{"date":"2026-10-22"},"end":{"date":"2026-10-23"}},
            {"id":"d","eventType":"default","summary":"Lunch","start":{"dateTime":"2026-10-21T12:00:00Z"},"end":{"dateTime":"2026-10-21T13:00:00Z"}}
            ]}"#,
        );
        let kinds: Vec<EventKind> = evs.iter().map(|e| e.kind).collect();
        assert_eq!(
            kinds,
            [
                EventKind::OutOfOffice,
                EventKind::FocusTime,
                EventKind::WorkingLocation,
                EventKind::WorkingLocation,
                EventKind::Regular
            ]
        );
        assert_eq!(evs[2].title, "Home");
        assert_eq!(evs[3].title, "King's Cross");
        assert!(!evs[0].is_regular() && evs[4].is_regular());
        assert_eq!(EventKind::FocusTime.label(), Some("Focus time"));
        assert_eq!(EventKind::Regular.label(), None);
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
    fn only_owners_and_writers_can_write_to_a_calendar() {
        let r: CalendarListResponse = serde_json::from_str(
            r##"{"items":[
            {"id":"a","accessRole":"owner"},
            {"id":"b","accessRole":"writer"},
            {"id":"c","accessRole":"reader"},
            {"id":"d","accessRole":"freeBusyReader"},
            {"id":"e"}
            ]}"##,
        )
        .unwrap();
        let writable: Vec<bool> = r
            .items
            .into_iter()
            .filter_map(|c| c.into_model("me@x"))
            .map(|c| c.writable)
            .collect();
        assert_eq!(writable, [true, true, false, false, true]);
        assert!(CALENDAR_FIELDS.contains("accessRole"));
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

    fn draft() -> EventDraft {
        EventDraft {
            title: "Lunch".into(),
            when: When::Timed {
                start: "2026-11-26T13:00:00Z".parse().unwrap(),
                end: "2026-11-26T14:00:00Z".parse().unwrap(),
            },
            time_zone: Some("Europe/London".into()),
            location: Some("Dishoom".into()),
            description: None,
            guests: vec![],
            meet: false,
            rrule: None,
            edit_guests: None,
            edit_reminders: None,
            series: None,
        }
    }

    #[test]
    fn insert_body_has_only_what_was_entered() {
        assert_eq!(
            insert_body(&draft(), "rid"),
            serde_json::json!({
                "summary": "Lunch",
                "start": {"dateTime": "2026-11-26T13:00:00Z", "timeZone": "Europe/London"},
                "end": {"dateTime": "2026-11-26T14:00:00Z", "timeZone": "Europe/London"},
                "location": "Dishoom",
            })
        );
    }

    #[test]
    fn insert_body_all_day_guests_meet_and_recurrence() {
        let mut d = draft();
        d.when = When::AllDay {
            start: date(2026, 11, 26),
            end: date(2026, 11, 27),
        };
        d.guests = vec!["a@example.com".into()];
        d.meet = true;
        d.rrule = Some("FREQ=WEEKLY;BYDAY=TH".into());
        let b = insert_body(&d, "rid");
        assert_eq!(b["start"], serde_json::json!({"date": "2026-11-26"}));
        assert_eq!(b["end"], serde_json::json!({"date": "2026-11-27"}));
        assert_eq!(b["attendees"][0]["email"], "a@example.com");
        assert_eq!(b["recurrence"][0], "RRULE:FREQ=WEEKLY;BYDAY=TH");
        assert_eq!(b["conferenceData"]["createRequest"]["requestId"], "rid");
    }

    #[test]
    fn patch_body_clears_fields_and_never_touches_attendees() {
        let mut d = draft();
        d.location = None;
        d.guests = vec!["a@example.com".into()];
        d.meet = true;
        d.rrule = Some("FREQ=DAILY".into());
        let b = patch_body(&d);
        assert!(b["location"].is_null());
        let obj = b.as_object().unwrap();
        // An unchanged description is left alone; an explicit empty one clears it.
        assert!(!obj.contains_key("description"));
        d.description = Some(String::new());
        assert!(patch_body(&d)["description"].is_null());
        let obj = b.as_object().unwrap();
        for key in ["attendees", "recurrence", "conferenceData", "description"] {
            assert!(!obj.contains_key(key), "{key} must not be patched");
        }
    }

    fn guest(email: &str, response: Response) -> Attendee {
        Attendee {
            name: email.into(),
            email: email.into(),
            display_name: None,
            comment: None,
            additional_guests: None,
            optional: None,
            resource: None,
            response,
            organizer: false,
            is_self: false,
        }
    }

    #[test]
    fn guest_edits_keep_rsvps_the_organiser_you_and_rooms() {
        let mut organiser = guest("boss@example.com", Response::Accepted);
        organiser.organizer = true;
        let mut me = guest("me@example.com", Response::Tentative);
        me.is_self = true;
        let mut room = guest("room@resource.calendar.google.com", Response::Accepted);
        room.resource = Some(true);
        let current = vec![
            organiser,
            me,
            room,
            guest("ann@example.com", Response::Accepted),
            guest("bob@example.com", Response::Declined),
        ];
        // Keep Ann, drop Bob, add Cy (case does not matter).
        let merged = merged_attendees(
            &current,
            &["Ann@Example.com".into(), "cy@example.com".into()],
        )
        .unwrap()
        .unwrap();
        let emails: Vec<&str> = merged
            .iter()
            .map(|v| v["email"].as_str().unwrap())
            .collect();
        assert_eq!(
            emails,
            [
                "boss@example.com",
                "me@example.com",
                "room@resource.calendar.google.com",
                "ann@example.com",
                "cy@example.com"
            ]
        );
        assert_eq!(merged[1]["responseStatus"], "tentative");
        assert_eq!(merged[3]["responseStatus"], "accepted");
        // A new guest carries no response, so Google marks them as needing one.
        assert!(merged[4].get("responseStatus").is_none());
        // Nothing to do when the list already matches.
        assert_eq!(
            merged_attendees(
                &current,
                &["ann@example.com".into(), "bob@example.com".into()]
            )
            .unwrap(),
            None
        );
        // Removing everyone keeps the organiser, you and rooms.
        assert_eq!(merged_attendees(&current, &[]).unwrap().unwrap().len(), 3);
    }

    #[test]
    fn reminder_edits_are_patched_only_when_asked() {
        let mut d = draft();
        assert!(patch_body(&d).get("reminders").is_none());
        d.edit_reminders = Some(RemindersEdit::Defaults);
        assert_eq!(
            patch_body(&d)["reminders"],
            serde_json::json!({"useDefault": true})
        );
        d.edit_reminders = Some(RemindersEdit::Custom(vec![10, 60]));
        assert_eq!(
            patch_body(&d)["reminders"],
            serde_json::json!({"useDefault": false, "overrides": [
                {"method": "popup", "minutes": 10},
                {"method": "popup", "minutes": 60}
            ]})
        );
        d.edit_reminders = Some(RemindersEdit::Custom(vec![]));
        assert_eq!(
            patch_body(&d)["reminders"],
            serde_json::json!({"useDefault": false, "overrides": []})
        );
    }

    /// Answers one HTTP request on `listener` with 200 and `body`; returns the request.
    fn serve_ok(listener: &std::net::TcpListener, body: &str) -> String {
        use std::io::{Read, Write};
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = Vec::new();
        loop {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            request.push(byte[0]);
            if request.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let head = String::from_utf8_lossy(&request).to_string();
        let len = head
            .lines()
            .find_map(|l| {
                let (n, v) = l.split_once(':')?;
                n.eq_ignore_ascii_case("content-length")
                    .then(|| v.trim().parse::<usize>().unwrap())
            })
            .unwrap_or(0);
        let mut payload = vec![0; len];
        stream.read_exact(&mut payload).unwrap();
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        stream.flush().unwrap();
        format!("{head}{}", String::from_utf8_lossy(&payload))
    }

    #[test]
    fn move_posts_to_the_move_endpoint_then_applies_the_edit() {
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base = format!("http://{}/calendar/v3", listener.local_addr().unwrap());
        let stored = r#"{"id":"e1","summary":"Lunch",
            "start":{"dateTime":"2026-11-26T13:00:00Z"},"end":{"dateTime":"2026-11-26T14:00:00Z"}}"#;
        let server = thread::spawn(move || {
            let moved = serve_ok(&listener, stored);
            let patched = serve_ok(&listener, stored);
            (moved, patched)
        });
        let mut to = cal();
        to.remote_id = "other@example.com".into();
        to.id = "me|other@example.com".into();
        let event =
            move_event_at(&agent(), "tok", &cal(), "e1", &to, Some(&draft()), &base).unwrap();
        assert_eq!(event.id, "e1");
        assert_eq!(event.calendar_id, to.id);
        let (moved, patched) = server.join().unwrap();
        assert!(moved.starts_with("POST /calendar/v3/calendars/"));
        assert!(moved.contains("/events/e1/move?"));
        assert!(moved.contains("destination=other%40example.com"));
        // The edit lands on the new calendar.
        assert!(patched.starts_with("PATCH /calendar/v3/calendars/other@example.com/events/e1"));

        let mut elsewhere = cal();
        elsewhere.account = "someone@else".into();
        assert!(matches!(
            move_event_at(&agent(), "tok", &cal(), "e1", &elsewhere, None, &base),
            Err(ApiError::InvalidRequest(_))
        ));
    }

    #[test]
    fn a_series_edit_patches_the_series_and_keeps_its_dates() {
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base = format!("http://{}/calendar/v3", listener.local_addr().unwrap());
        let master = r#"{"start":{"dateTime":"2026-10-05T09:30:00Z"},"end":{"dateTime":"2026-10-05T09:45:00Z"}}"#;
        let stored = r#"{"id":"series","summary":"Standup",
            "start":{"dateTime":"2026-10-05T10:00:00Z"},"end":{"dateTime":"2026-10-05T10:30:00Z"}}"#;
        let server = thread::spawn(move || {
            let get = serve_ok(&listener, master);
            let patch = serve_ok(&listener, stored);
            let plain = serve_ok(&listener, stored);
            (get, patch, plain)
        });
        // This occurrence is on 26 November; the editor moved it to 10:00-10:30.
        let mut d = draft();
        d.title = "Standup".into();
        d.when = When::Timed {
            start: "2026-11-26T10:00:00Z".parse().unwrap(),
            end: "2026-11-26T10:30:00Z".parse().unwrap(),
        };
        d.time_zone = Some("UTC".into());
        d.series = Some(SeriesEdit {
            change_time: true,
            following: false,
        });
        update_series_at(
            &agent(),
            "tok",
            &cal(),
            "series_20261126T093000Z",
            &d,
            &jiff::tz::TimeZone::UTC,
            &base,
        )
        .unwrap();
        d.series = Some(SeriesEdit {
            change_time: false,
            following: false,
        });
        update_series_at(
            &agent(),
            "tok",
            &cal(),
            "series_20261126T093000Z",
            &d,
            &jiff::tz::TimeZone::UTC,
            &base,
        )
        .unwrap();
        let (get, patch, plain) = server.join().unwrap();
        assert!(get.starts_with("GET /calendar/v3/calendars/"));
        assert!(get.contains("/events/series?"));
        assert!(patch.starts_with("PATCH /calendar/v3/calendars/"));
        assert!(patch.contains("/events/series?"));
        let body: serde_json::Value =
            serde_json::from_str(patch.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        // The series keeps its first date (5 October) and takes the new time of day and length.
        assert_eq!(body["start"]["dateTime"], "2026-10-05T10:00:00Z");
        assert_eq!(body["end"]["dateTime"], "2026-10-05T10:30:00Z");
        assert_eq!(body["summary"], "Standup");
        // Without a time change the series' own times are not sent.
        let body: serde_json::Value =
            serde_json::from_str(plain.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        assert!(body.get("start").is_none() && body.get("end").is_none());
    }

    #[test]
    fn instance_ids_give_the_cut_off_for_the_old_series() {
        let timed = instance_start("s_20261126T093000Z").unwrap();
        assert_eq!(until_before(timed).unwrap(), "20261126T092959Z");
        let all_day = instance_start("s_20261126").unwrap();
        assert_eq!(until_before(all_day).unwrap(), "20261125");
        assert!(instance_start("plain").is_none());
        assert_eq!(
            rule_without_end("FREQ=WEEKLY;UNTIL=20270101T000000Z;BYDAY=MO"),
            "FREQ=WEEKLY;BYDAY=MO"
        );
    }

    #[test]
    fn cutting_a_series_refuses_counted_rules_and_keeps_other_lines() {
        let lines = vec![
            "RRULE:FREQ=DAILY;UNTIL=20271231".to_string(),
            "EXDATE:20261201T093000Z".to_string(),
        ];
        let (cut, rule) = cut_recurrence(&lines, "20261125").unwrap();
        assert_eq!(cut[0], "RRULE:FREQ=DAILY;UNTIL=20261125");
        assert_eq!(cut[1], lines[1]);
        assert_eq!(rule, "FREQ=DAILY");
        let counted = vec!["RRULE:FREQ=DAILY;COUNT=10".to_string()];
        assert!(matches!(
            cut_recurrence(&counted, "20261125"),
            Err(ApiError::InvalidRequest(_))
        ));
        assert!(cut_recurrence(&[], "20261125").is_err());
    }

    #[test]
    fn editing_following_creates_a_new_series_then_cuts_the_old_one() {
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base = format!("http://{}/calendar/v3", listener.local_addr().unwrap());
        let master = r#"{"recurrence":["RRULE:FREQ=DAILY"],"description":"notes",
            "attendees":[{"email":"me@x","self":true,"organizer":true},{"email":"ann@x"}],
            "reminders":{"useDefault":false,"overrides":[{"method":"popup","minutes":5}]}}"#;
        let server = thread::spawn(move || {
            let get = serve_ok(&listener, master);
            let create = serve_ok(&listener, r#"{"id":"newseries"}"#);
            let patch = serve_ok(&listener, "{}");
            (get, create, patch)
        });
        let mut d = draft();
        d.title = "Standup".into();
        d.when = When::Timed {
            start: "2026-11-26T10:00:00Z".parse().unwrap(),
            end: "2026-11-26T10:30:00Z".parse().unwrap(),
        };
        d.time_zone = Some("UTC".into());
        d.series = Some(SeriesEdit {
            change_time: true,
            following: true,
        });
        update_series_at(
            &agent(),
            "tok",
            &cal(),
            "series_20261126T093000Z",
            &d,
            &jiff::tz::TimeZone::UTC,
            &base,
        )
        .unwrap();
        let (get, create, patch) = server.join().unwrap();
        assert!(get.starts_with("GET ") && get.contains("/events/series?"));
        assert!(create.starts_with("POST ") && create.contains("sendUpdates=all"));
        let body = |r: &str| -> serde_json::Value {
            serde_json::from_str(r.split("\r\n\r\n").nth(1).unwrap()).unwrap()
        };
        let new = body(&create);
        assert_eq!(new["recurrence"][0], "RRULE:FREQ=DAILY");
        assert_eq!(new["start"]["dateTime"], "2026-11-26T10:00:00Z");
        assert_eq!(new["description"], "notes");
        assert_eq!(new["attendees"].as_array().unwrap().len(), 1);
        assert_eq!(new["reminders"]["overrides"][0]["minutes"], 5);
        assert!(patch.starts_with("PATCH ") && patch.contains("/events/series "));
        assert_eq!(
            body(&patch)["recurrence"][0],
            "RRULE:FREQ=DAILY;UNTIL=20261126T092959Z"
        );
    }

    #[test]
    fn create_update_and_delete_round_trip_against_a_mock() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        fn serve(listener: &TcpListener, status: &str, body: &str) -> String {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            loop {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            let head = String::from_utf8_lossy(&request).to_string();
            let len = head
                .lines()
                .find_map(|l| {
                    let (n, v) = l.split_once(':')?;
                    n.eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse::<usize>().unwrap())
                })
                .unwrap_or(0);
            let mut payload = vec![0; len];
            stream.read_exact(&mut payload).unwrap();
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            stream.flush().unwrap();
            format!("{head}{}", String::from_utf8_lossy(&payload))
        }

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let base = format!("http://{}/calendar/v3", listener.local_addr().unwrap());
        let stored = r#"{"id":"new1","summary":"Lunch","location":"Dishoom",
            "start":{"dateTime":"2026-11-26T13:00:00Z"},"end":{"dateTime":"2026-11-26T14:00:00Z"}}"#;
        let server = thread::spawn(move || {
            let create = serve(&listener, "200 OK", stored);
            let update = serve(&listener, "200 OK", stored);
            let current = serve(
                &listener,
                "200 OK",
                r#"{"etag":"\"e1\"","attendees":[
                    {"email":"boss@example.com","organizer":true,"responseStatus":"accepted"},
                    {"email":"bob@example.com","responseStatus":"declined"}]}"#,
            );
            let guests = serve(&listener, "200 OK", stored);
            let delete = serve(
                &listener,
                "410 Gone",
                r#"{"error":{"message":"Resource has been deleted"}}"#,
            );
            (create, update, current, guests, delete)
        });

        let mut d = draft();
        d.meet = true;
        d.guests = vec!["a@example.com".into()];
        let created = create_event_at(&agent(), "tok", &cal(), &d, &base).unwrap();
        assert_eq!(created.id, "new1");
        assert_eq!(created.title, "Lunch");
        let updated = update_event_at(&agent(), "tok", &cal(), "new1", &d, &base).unwrap();
        assert_eq!(updated.id, "new1");
        let mut edit = draft();
        edit.edit_guests = Some(vec!["cy@example.com".into()]);
        update_event_at(&agent(), "tok", &cal(), "new1", &edit, &base).unwrap();
        delete_event_at(&agent(), "tok", &cal(), "new1", &base).unwrap();

        let (create, update, current, guests, delete) = server.join().unwrap();
        assert!(current.starts_with("GET /calendar/v3/calendars/"));
        assert!(guests.starts_with("PATCH /calendar/v3/calendars/"));
        assert!(guests.contains("sendUpdates=all"));
        assert!(guests.to_ascii_lowercase().contains("if-match: \"e1\""));
        // Bob goes, Cy comes, the organiser stays; the new guest has no response yet.
        let body: serde_json::Value =
            serde_json::from_str(guests.split("\r\n\r\n").nth(1).unwrap()).unwrap();
        let emails: Vec<&str> = body["attendees"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a["email"].as_str().unwrap())
            .collect();
        assert_eq!(emails, ["boss@example.com", "cy@example.com"]);
        assert!(!update.contains("sendUpdates"));
        let first = create.lines().next().unwrap();
        assert!(first.starts_with("POST /calendar/v3/calendars/"), "{first}");
        assert!(first.contains("conferenceDataVersion=1"));
        assert!(first.contains("sendUpdates=all"));
        assert!(
            create
                .to_ascii_lowercase()
                .contains("authorization: bearer tok")
        );
        assert!(update.starts_with("PATCH /calendar/v3/calendars/"));
        assert!(
            update
                .lines()
                .next()
                .unwrap()
                .contains("/events/new1?fields=")
        );
        assert!(delete.starts_with("DELETE /calendar/v3/calendars/"));
    }
}
