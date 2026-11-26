//! The command palette's logic: which commands exist, and which ones a typed query finds.
//!
//! Pure functions, so they are tested without a window. The UI lives in `ui/palette.rs`.

use jiff::Zoned;
use jiff::civil::Date;

use crate::config::ViewMode;
use crate::model::When;
use crate::quickadd;

/// How many matches the palette lists.
pub const MAX_SHOWN: usize = 8;

/// What running a palette entry does.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Today,
    Previous,
    Next,
    View(ViewMode),
    NewEvent,
    /// Open quick-add for a Google Task.
    NewTask,
    Refresh,
    Settings,
    GoTo(Date),
    /// Show only the calendars of the saved set with this name.
    ApplySet(String),
    /// Show or hide the calendar with this id.
    ToggleCalendar(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub label: String,
    /// A short note: the shortcut, or what the entry will do.
    pub detail: String,
    pub action: Action,
}

fn entry(label: impl Into<String>, detail: impl Into<String>, action: Action) -> Entry {
    Entry {
        label: label.into(),
        detail: detail.into(),
        action,
    }
}

/// A calendar as the palette needs it.
pub struct CalendarRef {
    pub id: String,
    pub name: String,
    pub visible: bool,
    /// A Google Tasks list rather than a calendar.
    pub task_list: bool,
}

/// Every fixed command, then one per saved set and one per calendar.
pub fn entries(sets: &[String], calendars: &[CalendarRef]) -> Vec<Entry> {
    let mut out = vec![
        entry("New event", "n", Action::NewEvent),
        entry("Go to today", "t", Action::Today),
        entry("Next period", "→", Action::Next),
        entry("Previous period", "←", Action::Previous),
        entry("Month view", "m", Action::View(ViewMode::Month)),
        entry("Week view", "w", Action::View(ViewMode::Week)),
        entry("Day view", "d", Action::View(ViewMode::Day)),
        entry("Agenda view", "a", Action::View(ViewMode::Agenda)),
        entry("Refresh", "Ctrl+R", Action::Refresh),
        entry("Settings", "Ctrl+,", Action::Settings),
    ];
    if calendars.iter().any(|c| c.task_list) {
        out.insert(1, entry("New task", "todo …", Action::NewTask));
    }
    out.extend(sets.iter().map(|s| {
        entry(
            format!("Show set: {s}"),
            "calendar set",
            Action::ApplySet(s.clone()),
        )
    }));
    out.extend(calendars.iter().map(|c| {
        entry(
            format!("{} {}", if c.visible { "Hide" } else { "Show" }, c.name),
            "calendar",
            Action::ToggleCalendar(c.id.clone()),
        )
    }));
    out
}

/// A date named by the query ("friday", "dec 25", "go to next monday"), as a jump target.
fn date_jump(query: &str, now: &Zoned) -> Option<Entry> {
    let lower = query.trim().to_lowercase();
    let text = ["go to ", "goto ", "jump to ", "go ", "jump "]
        .iter()
        .find_map(|p| lower.strip_prefix(p))
        .unwrap_or(&lower);
    let parsed = quickadd::parse(text, now);
    if !parsed.title.is_empty() {
        return None;
    }
    let date = match parsed.when? {
        When::AllDay { start, .. } => start,
        When::Timed { start, .. } => start.to_zoned(now.time_zone().clone()).date(),
    };
    Some(entry(
        format!("Go to {date}"),
        "jump to this date",
        Action::GoTo(date),
    ))
}

/// The entries that match `query`, best first. An empty query lists the first few commands.
/// Every word of the query must appear in the label; labels that start with it come first.
pub fn search(query: &str, all: &[Entry], now: &Zoned) -> Vec<Entry> {
    let q = query.trim().to_lowercase();
    let mut out = Vec::new();
    if q.is_empty() {
        out.extend(all.iter().take(MAX_SHOWN).cloned());
        return out;
    }
    let words: Vec<&str> = q.split_whitespace().collect();
    let mut scored: Vec<(usize, usize, &Entry)> = all
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let label = e.label.to_lowercase();
            let mut score = 0;
            for w in &words {
                let at = label.find(w)?;
                // A match at the start of a word beats one in the middle of it.
                let word_start = at == 0 || label.as_bytes()[at - 1] == b' ';
                score += at + if word_start { 0 } else { 100 };
            }
            Some((score, i, e))
        })
        .collect();
    scored.sort_by_key(|(score, i, _)| (*score, *i));
    out.extend(scored.into_iter().map(|(_, _, e)| e.clone()));
    if let Some(jump) = date_jump(query, now) {
        // A date that looks like a command word (a view or a day name) is still offered, last.
        out.push(jump);
    }
    out.truncate(MAX_SHOWN);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    fn now() -> Zoned {
        date(2026, 11, 25)
            .at(10, 0, 0, 0)
            .to_zoned(jiff::tz::TimeZone::UTC)
            .unwrap()
    }

    fn all() -> Vec<Entry> {
        entries(
            &["Work".to_string()],
            &[
                CalendarRef {
                    id: "a".into(),
                    name: "Birthdays".into(),
                    visible: true,
                    task_list: false,
                },
                CalendarRef {
                    id: "b".into(),
                    name: "Work".into(),
                    visible: false,
                    task_list: false,
                },
            ],
        )
    }

    #[test]
    fn new_task_is_offered_only_with_a_task_list() {
        assert!(!all().iter().any(|e| e.action == Action::NewTask));
        let with_tasks = entries(
            &[],
            &[CalendarRef {
                id: "t".into(),
                name: "My Tasks".into(),
                visible: true,
                task_list: true,
            }],
        );
        let found = search("task", &with_tasks, &now());
        assert_eq!(found[0].action, Action::NewTask);
    }

    #[test]
    fn an_empty_query_lists_the_first_commands() {
        let found = search("  ", &all(), &now());
        assert_eq!(found.len(), MAX_SHOWN);
        assert_eq!(found[0].action, Action::NewEvent);
    }

    #[test]
    fn words_match_anywhere_and_word_starts_win() {
        let found = search("view", &all(), &now());
        assert!(found.iter().all(|e| e.label.contains("view")));
        let found = search("wor", &all(), &now());
        // "Week view" has no "wor"; the set and the calendar do.
        let labels: Vec<_> = found.iter().map(|e| e.label.as_str()).collect();
        assert_eq!(labels, ["Show Work", "Show set: Work"]);
        // Both words must be present.
        assert_eq!(search("hide birth", &all(), &now()).len(), 1);
        assert!(search("hide work", &all(), &now()).is_empty());
    }

    #[test]
    fn calendars_offer_the_opposite_of_what_they_do_now() {
        let found = search("work", &all(), &now());
        assert!(
            found
                .iter()
                .any(|e| e.action == Action::ToggleCalendar("b".into()) && e.label == "Show Work")
        );
        let found = search("birthdays", &all(), &now());
        assert_eq!(found[0].label, "Hide Birthdays");
    }

    #[test]
    fn a_date_in_the_query_becomes_a_jump() {
        let found = search("go to dec 25", &all(), &now());
        assert_eq!(
            found.last().unwrap().action,
            Action::GoTo(date(2026, 12, 25))
        );
        let found = search("tomorrow", &all(), &now());
        assert_eq!(
            found.last().unwrap().action,
            Action::GoTo(date(2026, 11, 26))
        );
        // Plain words are not dates.
        assert!(
            search("zzz", &all(), &now()).is_empty(),
            "no command and no date"
        );
    }
}
