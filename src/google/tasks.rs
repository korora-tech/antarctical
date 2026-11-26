//! Google Tasks API v1. Each task list is shown as a "calendar" (see [`Calendar::is_task_list`])
//! and each task with a due date as an all-day [`Event`] of kind [`EventKind::Task`], so tasks
//! flow through the same cache, month fetches, views and optimistic writes as events.
//!
//! Google keeps only a date for a task's due time, so tasks are always all-day, and tasks
//! without a due date have no day to show on and are skipped.

use jiff::Timestamp;
use jiff::civil::Date;
use serde::Deserialize;

use super::api::{ApiError, EventDraft, get, send_write};
use crate::model::{Calendar, Event, EventKind, When, raw_task_title, task_title};

const BASE: &str = "https://tasks.googleapis.com/tasks/v1";
/// Tasks are shown in this colour (Google's Tasks blue); a list has no colour of its own.
const TASK_COLOR: u32 = 0x4285f4;
const TASK_FIELDS: &str = "items(id,title,notes,status,due,webViewLink,deleted),nextPageToken";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListsResponse {
    #[serde(default)]
    items: Vec<WireList>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct WireList {
    id: String,
    #[serde(default)]
    title: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TasksResponse {
    #[serde(default)]
    items: Vec<WireTask>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireTask {
    id: String,
    #[serde(default)]
    title: String,
    notes: Option<String>,
    #[serde(default)]
    status: String,
    due: Option<String>,
    web_view_link: Option<String>,
    #[serde(default)]
    deleted: bool,
}

impl WireTask {
    fn into_model(self, list: &Calendar) -> Option<Event> {
        if self.deleted {
            return None;
        }
        let start = due_date(self.due.as_deref()?)?;
        let end = start.tomorrow().unwrap_or(start);
        let done = self.status == "completed";
        let raw = if self.title.trim().is_empty() {
            "(No title)"
        } else {
            self.title.trim()
        };
        Some(Event {
            id: self.id,
            calendar_id: list.id.clone(),
            title: task_title(raw, done),
            when: When::AllDay { start, end },
            color: None,
            location: None,
            description: self.notes.filter(|n| !n.trim().is_empty()),
            html_link: self.web_view_link,
            meet_link: None,
            organizer: None,
            attendees: vec![],
            reminders: vec![],
            tentative: false,
            kind: EventKind::Task { done },
            time_zone: None,
        })
    }
}

/// The civil date of a task's `due` (an RFC 3339 timestamp at midnight UTC): its date part, as
/// Google's own apps show it, whatever zone this machine is in.
fn due_date(due: &str) -> Option<Date> {
    due.get(..10)?.parse().ok()
}

/// The `due` Google expects for `date`.
fn due_string(date: Date) -> String {
    format!("{date}T00:00:00.000Z")
}

/// The task lists of `account` as calendars, in Google's order.
pub fn task_lists(
    agent: &ureq::Agent,
    token: &str,
    account: &str,
) -> Result<Vec<Calendar>, ApiError> {
    let mut out = vec![];
    let mut page: Option<String> = None;
    loop {
        let mut url = url::Url::parse(&format!("{BASE}/users/@me/lists"))
            .expect("BASE and the lists path form a valid Google API URL");
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("maxResults", "100")
                .append_pair("fields", "items(id,title),nextPageToken");
            if let Some(p) = &page {
                q.append_pair("pageToken", p);
            }
        }
        let resp: ListsResponse = get(agent, &url, token)?;
        out.extend(resp.items.into_iter().map(|list| Calendar {
            id: Calendar::key(account, &Calendar::task_list_remote_id(&list.id)),
            account: account.to_string(),
            remote_id: Calendar::task_list_remote_id(&list.id),
            name: if list.title.trim().is_empty() {
                "Tasks".into()
            } else {
                list.title
            },
            color: TASK_COLOR,
            primary: false,
            default_reminders: vec![],
            selected_upstream: true,
            writable: true,
        }));
        match resp.next_page_token {
            Some(p) => page = Some(p),
            None => break,
        }
    }
    Ok(out)
}

fn list_id(list: &Calendar) -> Result<&str, ApiError> {
    list.task_list_id()
        .ok_or_else(|| ApiError::InvalidRequest("this is not a task list".into()))
}

fn tasks_url(list: &Calendar, task: Option<&str>) -> Result<url::Url, ApiError> {
    let mut url = url::Url::parse(BASE).expect("BASE is a valid static Google Tasks API URL");
    {
        let mut segments = url
            .path_segments_mut()
            .expect("BASE has no query or fragment, so its path segments are mutable");
        segments.push("lists").push(list_id(list)?).push("tasks");
        if let Some(task) = task {
            segments.push(task);
        }
    }
    Ok(url)
}

/// The tasks of `list` due in `[from, to)`, done or not.
pub fn tasks(
    agent: &ureq::Agent,
    token: &str,
    list: &Calendar,
    from: Timestamp,
    to: Timestamp,
) -> Result<Vec<Event>, ApiError> {
    let mut out = vec![];
    let mut page: Option<String> = None;
    loop {
        let mut url = tasks_url(list, None)?;
        {
            let mut q = url.query_pairs_mut();
            q.append_pair("showCompleted", "true")
                .append_pair("showHidden", "true")
                .append_pair("dueMin", &from.to_string())
                .append_pair("dueMax", &to.to_string())
                .append_pair("maxResults", "100")
                .append_pair("fields", TASK_FIELDS);
            if let Some(p) = &page {
                q.append_pair("pageToken", p);
            }
        }
        let resp: TasksResponse = get(agent, &url, token)?;
        out.extend(resp.items.into_iter().filter_map(|t| t.into_model(list)));
        match resp.next_page_token {
            Some(p) => page = Some(p),
            None => break,
        }
    }
    Ok(out)
}

fn day_of(draft: &EventDraft) -> Date {
    match draft.when {
        When::AllDay { start, .. } => start,
        When::Timed { start, .. } => start.to_zoned(jiff::tz::TimeZone::system()).date(),
    }
}

fn parse_written(body: &str, list: &Calendar) -> Result<Event, ApiError> {
    let wire: WireTask = serde_json::from_str(body).map_err(|e| ApiError::Decode(e.to_string()))?;
    wire.into_model(list)
        .ok_or_else(|| ApiError::Decode("Google returned a task the app cannot display".into()))
}

/// Creates a task due on the draft's day.
pub fn create(
    agent: &ureq::Agent,
    token: &str,
    list: &Calendar,
    draft: &EventDraft,
) -> Result<Event, ApiError> {
    let url = tasks_url(list, None)?;
    let mut body = serde_json::json!({
        "title": raw_task_title(&draft.title),
        "due": due_string(day_of(draft)),
    });
    if let Some(notes) = draft.description.as_deref().filter(|n| !n.is_empty()) {
        body["notes"] = notes.into();
    }
    let text = send_write(agent, "POST", &url, token, Some(&body), None)?;
    parse_written(&text, list)
}

/// Marks a task done, or not done again.
pub fn set_done(
    agent: &ureq::Agent,
    token: &str,
    list: &Calendar,
    task_id: &str,
    done: bool,
) -> Result<Event, ApiError> {
    let url = tasks_url(list, Some(task_id))?;
    let body = if done {
        serde_json::json!({"status": "completed"})
    } else {
        // Clearing `completed` is what makes Google forget when it was done.
        serde_json::json!({"status": "needsAction", "completed": null})
    };
    let text = send_write(agent, "PATCH", &url, token, Some(&body), None)?;
    parse_written(&text, list)
}

/// Deletes a task. A task that is already gone counts as deleted.
pub fn delete(
    agent: &ureq::Agent,
    token: &str,
    list: &Calendar,
    task_id: &str,
) -> Result<(), ApiError> {
    let url = tasks_url(list, Some(task_id))?;
    match send_write(agent, "DELETE", &url, token, None, None) {
        Ok(_) | Err(ApiError::NotFound) => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn list() -> Calendar {
        Calendar {
            id: "me@x/tasks:L1".into(),
            account: "me@x".into(),
            remote_id: "tasks:L1".into(),
            name: "My Tasks".into(),
            color: TASK_COLOR,
            primary: false,
            default_reminders: vec![],
            selected_upstream: true,
            writable: true,
        }
    }

    fn parse(json: &str) -> Vec<Event> {
        let resp: TasksResponse = serde_json::from_str(json).unwrap();
        resp.items
            .into_iter()
            .filter_map(|t| t.into_model(&list()))
            .collect()
    }

    #[test]
    fn a_task_becomes_an_all_day_event_on_its_due_date() {
        let events = parse(
            r#"{"items":[
              {"id":"a","title":"Buy milk","status":"needsAction","due":"2026-11-27T00:00:00.000Z","notes":"2 litres"},
              {"id":"b","title":"File taxes","status":"completed","due":"2026-11-28T00:00:00.000Z"}
            ]}"#,
        );
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].title, "☐ Buy milk");
        assert_eq!(
            events[0].when,
            When::AllDay {
                start: date(2026, 11, 27),
                end: date(2026, 11, 28)
            }
        );
        assert_eq!(events[0].description.as_deref(), Some("2 litres"));
        assert_eq!(events[0].kind, EventKind::Task { done: false });
        assert_eq!(events[0].calendar_id, "me@x/tasks:L1");
        assert_eq!(events[1].title, "☑ File taxes");
        assert!(events[1].is_done());
    }

    #[test]
    fn tasks_without_a_due_date_or_that_were_deleted_are_skipped() {
        let events = parse(
            r#"{"items":[
              {"id":"a","title":"Someday","status":"needsAction"},
              {"id":"b","title":"Gone","status":"needsAction","due":"2026-11-27T00:00:00.000Z","deleted":true},
              {"id":"c","status":"needsAction","due":"2026-11-27T00:00:00.000Z"}
            ]}"#,
        );
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].title, "☐ (No title)");
    }

    #[test]
    fn the_due_date_ignores_the_time_zone_of_this_machine() {
        assert_eq!(
            due_date("2026-11-27T00:00:00.000Z"),
            Some(date(2026, 11, 27))
        );
        assert_eq!(due_date("nonsense"), None);
        assert_eq!(due_string(date(2026, 1, 5)), "2026-01-05T00:00:00.000Z");
    }

    #[test]
    fn a_task_list_is_recognised_by_its_remote_id() {
        let l = list();
        assert!(l.is_task_list());
        assert_eq!(l.task_list_id(), Some("L1"));
        assert_eq!(Calendar::task_list_remote_id("L1"), l.remote_id);
    }

    #[test]
    fn task_titles_round_trip_their_marker() {
        assert_eq!(raw_task_title(&task_title("Buy milk", true)), "Buy milk");
        assert_eq!(raw_task_title(&task_title("Buy milk", false)), "Buy milk");
        assert_eq!(raw_task_title("Plain"), "Plain");
    }
}
