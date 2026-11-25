//! Event entry: the quick-add bar (one line of natural language, live preview) and the editor for
//! an existing event. Both are modal overlays on `MainView`.
//!
//! The pure parts (turning text into a draft, picking the calendar, round-tripping a time
//! through text) are plain functions at the top so they can be tested without a window.

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    MouseButton, SharedString, Subscription, Window, div, px, rgb,
};
use jiff::tz::TimeZone;
use jiff::{ToSpan, Zoned};

use super::format;
use super::text_input::{Submit, TextInput};
use super::theme::{AMBER, CAPTION, MUTED, PANEL, SECONDARY, SELECTED_BORDER, TEXT};
use super::widgets::{ButtonStyle, button};
use crate::google::api::EventDraft;
use crate::google::api::RemindersEdit;
use crate::google::api::SeriesEdit;
use crate::model::{Calendar, Event, When};
use crate::quickadd::{self, DEFAULT_DURATION_MINUTES, Parsed};
use crate::store::Store;
use jiff::civil::Date;

pub enum ComposeEvent {
    Close,
    /// The user asked to turn on editing for this account.
    OpenSettings,
}

// ----- pure logic ---------------------------------------------------------------------------

/// What pressing Enter in the quick-add bar would do.
#[derive(Debug, PartialEq)]
pub struct Resolved {
    pub draft: EventDraft,
    /// The calendar the event goes to.
    pub calendar: Calendar,
    /// Set when the event cannot be created as typed; Enter does nothing.
    pub problem: Option<Problem>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Problem {
    EmptyTitle,
    /// The account has not turned on editing (the events scope).
    EditingOff(String),
    /// The calendar is read-only for you (a holiday calendar, a shared calendar you can only view).
    ReadOnly(String),
    /// Recurrence needs an IANA zone name, which this machine did not report.
    NoZoneName,
}

impl Problem {
    pub fn message(&self) -> String {
        match self {
            Problem::EmptyTitle => "Type a title".into(),
            Problem::EditingOff(account) => {
                format!("Editing is off for {account}. Turn it on in Settings to add events.")
            }
            Problem::ReadOnly(name) => {
                format!("{name} is read-only for you. Pick another calendar with /name.")
            }
            Problem::NoZoneName => {
                "Repeating events need a named time zone, and this system did not report one."
                    .into()
            }
        }
    }
}

/// The slot used when the text names no date or time: the next full hour today, or 9:00 on the
/// day being viewed.
pub fn default_when(anchor: Date, now: &Zoned) -> When {
    let tz = now.time_zone().clone();
    let start = if anchor == now.date() {
        now.round(
            jiff::ZonedRound::new()
                .smallest(jiff::Unit::Hour)
                .mode(jiff::RoundMode::Ceil),
        )
        .unwrap_or_else(|_| now.clone())
    } else {
        anchor
            .at(9, 0, 0, 0)
            .to_zoned(tz)
            .unwrap_or_else(|_| now.clone())
    };
    let end = start
        .checked_add(DEFAULT_DURATION_MINUTES.minutes())
        .unwrap_or_else(|_| start.clone());
    When::Timed {
        start: start.timestamp(),
        end: end.timestamp(),
    }
}

/// The calendar a `/hint` names (case-insensitive substring of the name), else the primary
/// calendar of the first account that can edit, else of the first account.
pub fn pick_calendar<'a>(
    calendars: &'a [Calendar],
    accounts: &[String],
    hint: Option<&str>,
    can_edit: impl Fn(&str) -> bool,
) -> Option<&'a Calendar> {
    if let Some(h) = hint.map(str::to_lowercase).filter(|h| !h.is_empty())
        && let Some(c) = calendars
            .iter()
            .filter(|c| can_edit(&c.account) && c.writable)
            .chain(calendars.iter())
            .find(|c| c.name.to_lowercase().contains(&h))
    {
        return Some(c);
    }
    let rank = |a: &str| accounts.iter().position(|x| x == a).unwrap_or(usize::MAX);
    let mut accts: Vec<&str> = calendars.iter().map(|c| c.account.as_str()).collect();
    accts.sort_by_key(|a| rank(a));
    accts.dedup();
    let account = accts
        .iter()
        .find(|a| can_edit(a))
        .or_else(|| accts.first())?;
    let of_account = || calendars.iter().filter(|c| c.account == *account);
    of_account()
        .find(|c| c.primary && c.writable)
        .or_else(|| of_account().find(|c| c.writable))
        .or_else(|| of_account().find(|c| c.primary))
        .or_else(|| of_account().next())
}

/// Turns parsed text into a draft and its target calendar.
pub fn resolve(
    parsed: Parsed,
    anchor: Date,
    now: &Zoned,
    calendars: &[Calendar],
    accounts: &[String],
    can_edit: impl Fn(&str) -> bool,
) -> Option<Resolved> {
    let calendar = pick_calendar(
        calendars,
        accounts,
        parsed.calendar_hint.as_deref(),
        &can_edit,
    )?
    .clone();
    let when = parsed.when.unwrap_or_else(|| default_when(anchor, now));
    let tz_name = now.time_zone().iana_name().map(str::to_string);
    let problem = if parsed.title.trim().is_empty() {
        Some(Problem::EmptyTitle)
    } else if !can_edit(&calendar.account) {
        Some(Problem::EditingOff(calendar.account.clone()))
    } else if !calendar.writable {
        Some(Problem::ReadOnly(calendar.name.clone()))
    } else if parsed.recurrence.is_some() && tz_name.is_none() {
        Some(Problem::NoZoneName)
    } else {
        None
    };
    Some(Resolved {
        draft: EventDraft {
            title: parsed.title,
            when,
            time_zone: tz_name,
            location: parsed.location,
            description: None,
            guests: parsed.guests,
            meet: parsed.meet,
            rrule: parsed.recurrence.map(|r| r.rrule),
            edit_guests: None,
            edit_reminders: None,
            series: None,
        },
        calendar,
        problem,
    })
}

/// The guests shown in the editor: everyone but you, the organiser and rooms.
fn guest_emails(event: &Event) -> Vec<String> {
    event
        .attendees
        .iter()
        .filter(|a| !a.is_self && !a.organizer && a.resource != Some(true))
        .map(|a| a.email.to_lowercase())
        .collect()
}

/// Reads the editor's guests field: emails separated by commas, semicolons or spaces.
fn parse_guests(text: &str) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = vec![];
    for token in text
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter(|t| !t.is_empty())
    {
        let valid = token.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && domain.contains('.') && !domain.contains('@')
        });
        if !valid {
            return Err(format!("\"{token}\" is not an email address"));
        }
        let email = token.to_lowercase();
        if !out.contains(&email) {
            out.push(email);
        }
    }
    Ok(out)
}

fn same_emails(a: &[String], b: &[String]) -> bool {
    let sorted = |v: &[String]| {
        let mut v: Vec<String> = v.iter().map(|e| e.to_lowercase()).collect();
        v.sort();
        v.dedup();
        v
    };
    sorted(a) == sorted(b)
}

fn same_minutes(a: &[i64], b: &[i64]) -> bool {
    let sorted = |v: &[i64]| {
        let mut v = v.to_vec();
        v.sort();
        v.dedup();
        v
    };
    sorted(a) == sorted(b)
}

/// Reminders as the editor shows them: `10m, 1h, 1d`.
fn reminders_to_text(minutes: &[i64]) -> String {
    let mut sorted = minutes.to_vec();
    sorted.sort();
    sorted
        .into_iter()
        .map(|m| match m {
            m if m > 0 && m % 10080 == 0 => format!("{}w", m / 10080),
            m if m > 0 && m % 1440 == 0 => format!("{}d", m / 1440),
            m if m > 0 && m % 60 == 0 => format!("{}h", m / 60),
            m => format!("{m}m"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The longest lead time Google accepts: four weeks.
const MAX_REMINDER_MINUTES: i64 = 40_320;

/// Reads the editor's reminders field: `10m, 1h, 1d` (a bare number is minutes), `default` for the
/// calendar's own, or `none`.
fn text_to_reminders(text: &str) -> Result<RemindersEdit, String> {
    let text = text.trim().to_lowercase();
    match text.as_str() {
        "default" | "defaults" => return Ok(RemindersEdit::Defaults),
        "" | "none" | "off" => return Ok(RemindersEdit::Custom(vec![])),
        _ => {}
    }
    let mut out: Vec<i64> = vec![];
    for token in text
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter(|t| !t.is_empty())
    {
        let split = token
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(token.len());
        let (digits, unit) = token.split_at(split);
        let n: i64 = digits
            .parse()
            .map_err(|_| format!("\"{token}\" is not a reminder time; try 10m, 1h or 1d"))?;
        let factor = match unit {
            "" | "m" | "min" | "mins" => 1,
            "h" | "hr" | "hrs" => 60,
            "d" | "day" | "days" => 1440,
            "w" | "wk" | "week" | "weeks" => 10080,
            _ => {
                return Err(format!(
                    "\"{token}\" is not a reminder time; try 10m, 1h or 1d"
                ));
            }
        };
        let minutes = n * factor;
        if minutes > MAX_REMINDER_MINUTES {
            return Err("Reminders can be at most 4 weeks before the event".into());
        }
        if !out.contains(&minutes) {
            out.push(minutes);
        }
    }
    out.sort();
    Ok(RemindersEdit::Custom(out))
}

/// The editor's "when" field: text that `quickadd::parse` reads back to the same time, or `None`
/// when the event cannot be expressed that way (multi-day, zero-length).
pub fn when_to_text(when: &When, tz: &TimeZone) -> Option<String> {
    match when {
        When::AllDay { start, end } => {
            (end.checked_sub(1.day()).ok()? <= *start).then(|| start.to_string())
        }
        When::Timed { start, end } => {
            let s = start.to_zoned(tz.clone());
            let e = end.to_zoned(tz.clone());
            if start >= end || *end > start.checked_add(24.hours()).ok()? {
                return None;
            }
            Some(format!(
                "{} {}-{}",
                s.date(),
                s.strftime("%H:%M"),
                e.strftime("%H:%M")
            ))
        }
    }
}

/// Reads the editor's "when" field. `Err` carries what was not understood.
pub fn text_to_when(text: &str, now: &Zoned) -> Result<When, String> {
    let parsed = quickadd::parse(text, now);
    if !parsed.title.trim().is_empty() {
        return Err(format!("Did not understand \"{}\"", parsed.title.trim()));
    }
    parsed
        .when
        .ok_or_else(|| "Enter a date and time, like \"tomorrow 2-3pm\"".to_string())
}

// ----- shared view bits ---------------------------------------------------------------------

fn backdrop() -> gpui::Div {
    div()
        .absolute()
        .top(px(0.0))
        .left(px(0.0))
        .size_full()
        .flex()
        .flex_col()
        .items_center()
        .pt(px(96.0))
        .bg(gpui::rgba(0x0000_00a0))
}

fn panel() -> gpui::Div {
    div()
        .w(px(560.0))
        .max_w_full()
        .flex()
        .flex_col()
        .gap_3()
        .rounded_lg()
        .border_1()
        .border_color(rgb(SELECTED_BORDER))
        .bg(rgb(PANEL))
        .p_4()
        .text_color(rgb(TEXT))
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

fn fact(label: &'static str, value: String, color: u32) -> gpui::Div {
    div()
        .flex()
        .items_baseline()
        .gap(px(10.0))
        .text_size(px(12.0))
        .child(
            div()
                .w(px(64.0))
                .flex_shrink_0()
                .text_color(rgb(CAPTION))
                .child(label),
        )
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .text_color(rgb(color))
                .child(value),
        )
}

fn hint(text: &'static str) -> gpui::Div {
    div()
        .text_size(px(11.0))
        .text_color(rgb(CAPTION))
        .child(text)
}

// ----- quick add ----------------------------------------------------------------------------

pub struct QuickAddView {
    store: Entity<Store>,
    input: Entity<TextInput>,
    anchor: Date,
    /// A time chosen by dragging in the grid; text that names its own time wins.
    when: Option<When>,
    _subscription: Subscription,
}

impl EventEmitter<ComposeEvent> for QuickAddView {}

impl QuickAddView {
    pub fn new(
        store: Entity<Store>,
        anchor: Date,
        when: Option<When>,
        cx: &mut Context<Self>,
    ) -> Self {
        let input =
            cx.new(|cx| TextInput::new("Lunch with Sam tomorrow 1pm at Dishoom", false, 1, cx));
        let subscription = cx.observe(&input, |_, _, cx| cx.notify());
        Self {
            store,
            input,
            anchor,
            when,
            _subscription: subscription,
        }
    }

    pub fn first_focus(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }

    fn resolved(&self, cx: &App) -> Option<Resolved> {
        let store = self.store.read(cx);
        let text = self.input.read(cx).text().to_string();
        if text.trim().is_empty() && self.when.is_none() {
            return None;
        }
        let now = Zoned::now();
        let mut parsed = quickadd::parse(&text, &now);
        // A time picked by dragging in the grid, unless the text says otherwise.
        parsed.when = parsed.when.or(self.when);
        resolve(
            parsed,
            self.anchor,
            &now,
            &store.calendars,
            &store.accounts,
            |account| store.can_edit(account),
        )
    }

    fn submit(&mut self, cx: &mut Context<Self>) {
        let Some(r) = self.resolved(cx) else { return };
        if r.problem.is_some() {
            cx.notify();
            return;
        }
        self.store.update(cx, |store, cx| {
            store.create_event(&r.calendar, r.draft);
            cx.notify();
        });
        cx.emit(ComposeEvent::Close);
    }
}

impl Focusable for QuickAddView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input.focus_handle(cx)
    }
}

impl Render for QuickAddView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let resolved = self.resolved(cx);
        let store = self.store.read(cx);
        let clock = store.prefs.clock_24h;
        let tz = TimeZone::system();

        let preview: AnyElement = match &resolved {
            None if store.calendars.is_empty() => div()
                .text_size(px(12.0))
                .text_color(rgb(AMBER))
                .child("Sign in to a Google account first.")
                .into_any_element(),
            None => div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(hint("Try: Lunch with Sam tomorrow 1pm at Dishoom"))
                .child(hint(
                    "Standup every weekday 9:30am · Dentist fri 3-4pm /personal",
                ))
                .child(hint(
                    "Add guests by email, +meet for a video call, /name picks the calendar",
                ))
                .into_any_element(),
            Some(r) => {
                let ev = Store::event_from_draft(&r.calendar, "preview".into(), &r.draft);
                let cal_label = if store.accounts.len() > 1 && r.calendar.name != r.calendar.account
                {
                    format!("{} · {}", r.calendar.name, r.calendar.account)
                } else {
                    r.calendar.name.clone()
                };
                let mut rows = div().flex().flex_col().gap(px(6.0));
                rows = rows.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.0))
                        .child(
                            div()
                                .size(px(8.0))
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(rgb(r.calendar.color)),
                        )
                        .child(
                            div()
                                .text_size(px(15.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(if r.draft.title.is_empty() {
                                    "(No title)".to_string()
                                } else {
                                    r.draft.title.clone()
                                }),
                        ),
                );
                rows = rows.child(fact(
                    "When",
                    format::event_when_long(&ev, &tz, clock),
                    SECONDARY,
                ));
                if let Some(l) = &r.draft.location {
                    rows = rows.child(fact("Where", l.clone(), SECONDARY));
                }
                if !r.draft.guests.is_empty() {
                    rows = rows.child(fact("Guests", r.draft.guests.join(", "), SECONDARY));
                }
                if let Some(rrule) = &r.draft.rrule {
                    let label = quickadd::parse(self.input.read(cx).text(), &Zoned::now())
                        .recurrence
                        .map(|rec| rec.label)
                        .unwrap_or_else(|| rrule.clone());
                    rows = rows.child(fact("Repeats", label, SECONDARY));
                }
                if r.draft.meet {
                    rows = rows.child(fact("Video", "Google Meet link".into(), SECONDARY));
                }
                rows = rows.child(fact("Calendar", cal_label, MUTED));
                if let Some(p) = &r.problem {
                    rows = rows.child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(AMBER))
                            .child(p.message()),
                    );
                } else if !r.draft.guests.is_empty() {
                    rows = rows.child(hint("Guests are emailed an invitation."));
                }
                rows.into_any_element()
            }
        };

        let editing_off = matches!(
            resolved.as_ref().and_then(|r| r.problem.as_ref()),
            Some(Problem::EditingOff(_))
        );
        let can_add = resolved.as_ref().is_some_and(|r| r.problem.is_none());

        backdrop()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(ComposeEvent::Close)),
            )
            .child(
                panel()
                    .on_action(cx.listener(|this, _: &Submit, _, cx| this.submit(cx)))
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(SECONDARY))
                            .child("New event"),
                    )
                    .child(self.input.clone())
                    .child(preview)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(hint("Enter to add · Esc to cancel"))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .when(editing_off, |d| {
                                        d.child(button(
                                            "quickadd-settings",
                                            "Open Settings",
                                            ButtonStyle::Secondary,
                                            Some(Box::new(cx.listener(
                                                |_, _: &ClickEvent, _, cx| {
                                                    cx.emit(ComposeEvent::OpenSettings)
                                                },
                                            ))),
                                        ))
                                    })
                                    .child(button(
                                        "quickadd-add",
                                        "Add event",
                                        ButtonStyle::Primary,
                                        can_add.then(|| {
                                            Box::new(cx.listener(|this, _: &ClickEvent, _, cx| {
                                                this.submit(cx)
                                            }))
                                                as super::widgets::OnClick
                                        }),
                                    )),
                            ),
                    ),
            )
    }
}

// ----- editor -------------------------------------------------------------------------------

pub struct EditorView {
    store: Entity<Store>,
    calendar_id: String,
    event_id: String,
    title: Entity<TextInput>,
    when: Entity<TextInput>,
    location: Entity<TextInput>,
    guests: Entity<TextInput>,
    reminders: Entity<TextInput>,
    /// Guests (lower-case emails) the event had when the editor opened.
    original_guests: Vec<String>,
    original_reminders: Vec<i64>,
    /// Only the organiser can change who is invited.
    can_edit_guests: bool,
    /// Reminders are only offered on timed events.
    can_edit_reminders: bool,
    /// The time cannot be written as text (multi-day or zero-length): leave it as it is.
    when_locked: bool,
    original_when: When,
    confirm_delete: bool,
    /// The calendar the event will be saved to; differs from `calendar_id` after a pick.
    target_calendar_id: String,
    /// Moving needs a plain event you organise (not one occurrence of a series).
    can_move: bool,
    /// The event is one occurrence of a series you organise, so the edit can cover them all.
    in_series: bool,
    /// The edit and delete apply to the whole series.
    whole_series: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ComposeEvent> for EditorView {}

impl EditorView {
    pub fn new(store: Entity<Store>, event: &Event, cx: &mut Context<Self>) -> Self {
        let tz = TimeZone::system();
        let when_text = when_to_text(&event.when, &tz);
        let title = cx.new(|cx| TextInput::new("Title", false, 1, cx));
        let when = cx.new(|cx| TextInput::new("tomorrow 2-3pm", false, 2, cx));
        let location = cx.new(|cx| TextInput::new("Location", false, 3, cx));
        let guests = cx.new(|cx| TextInput::new("ann@example.com, bob@example.com", false, 4, cx));
        let reminders = cx.new(|cx| TextInput::new("10m, 1h, 1d · default · none", false, 5, cx));
        let original_guests = guest_emails(event);
        let original_reminders = event.reminders.clone();
        guests.update(cx, |i, cx| i.set_text(original_guests.join(", "), cx));
        reminders.update(cx, |i, cx| {
            i.set_text(reminders_to_text(&original_reminders), cx)
        });
        title.update(cx, |i, cx| i.set_text(event.title.clone(), cx));
        when.update(cx, |i, cx| {
            i.set_text(when_text.clone().unwrap_or_default(), cx)
        });
        location.update(cx, |i, cx| {
            i.set_text(event.location.clone().unwrap_or_default(), cx)
        });
        let subs = [&title, &when, &location, &guests, &reminders]
            .into_iter()
            .map(|e| cx.observe(e, |_, _, cx| cx.notify()))
            .collect();
        Self {
            store,
            calendar_id: event.calendar_id.clone(),
            event_id: event.id.clone(),
            title,
            when,
            location,
            guests,
            reminders,
            original_guests,
            original_reminders,
            can_edit_guests: event.user_organizes(),
            can_edit_reminders: matches!(event.when, When::Timed { .. }),
            when_locked: when_text.is_none(),
            original_when: event.when,
            confirm_delete: false,
            target_calendar_id: event.calendar_id.clone(),
            can_move: event.user_organizes() && !event.is_recurring_instance(),
            in_series: event.user_organizes() && event.is_recurring_instance(),
            whole_series: false,
            error: None,
            _subscriptions: subs,
        }
    }

    /// The calendars this event can move to: writable ones of the same account.
    fn move_targets(&self, cx: &App) -> Vec<Calendar> {
        let store = self.store.read(cx);
        let Some(current) = store.calendar(&self.calendar_id) else {
            return vec![];
        };
        store
            .calendars
            .iter()
            .filter(|c| c.account == current.account && store.can_write(c))
            .cloned()
            .collect()
    }

    fn toggle_scope(&mut self, cx: &mut Context<Self>) {
        self.whole_series = !self.whole_series;
        self.confirm_delete = false;
        cx.notify();
    }

    /// Picks the next calendar in the list.
    fn cycle_calendar(&mut self, cx: &mut Context<Self>) {
        let targets = self.move_targets(cx);
        if let Some(i) = targets.iter().position(|c| c.id == self.target_calendar_id) {
            self.target_calendar_id = targets[(i + 1) % targets.len()].id.clone();
            cx.notify();
        }
    }

    pub fn first_focus(&self, cx: &App) -> FocusHandle {
        self.title.focus_handle(cx)
    }

    fn parsed_when(&self, cx: &App) -> Result<When, String> {
        if self.when_locked {
            return Ok(self.original_when);
        }
        text_to_when(self.when.read(cx).text(), &Zoned::now())
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let title = self.title.read(cx).text().trim().to_string();
        if title.is_empty() {
            self.error = Some("Type a title".into());
            cx.notify();
            return;
        }
        let when = match self.parsed_when(cx) {
            Ok(w) => w,
            Err(e) => {
                self.error = Some(e);
                cx.notify();
                return;
            }
        };
        let location = self.location.read(cx).text().trim().to_string();
        let edit_guests = if self.can_edit_guests {
            match parse_guests(self.guests.read(cx).text()) {
                Ok(list) if same_emails(&list, &self.original_guests) => None,
                Ok(list) => Some(list),
                Err(e) => {
                    self.error = Some(e);
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let defaults = self
            .store
            .read(cx)
            .calendar(&self.calendar_id)
            .map(|c| c.default_reminders.clone())
            .unwrap_or_default();
        let edit_reminders = if self.can_edit_reminders {
            match text_to_reminders(self.reminders.read(cx).text()) {
                Ok(RemindersEdit::Custom(list))
                    if same_minutes(&list, &self.original_reminders) =>
                {
                    None
                }
                Ok(RemindersEdit::Defaults)
                    if same_minutes(&defaults, &self.original_reminders) =>
                {
                    None
                }
                Ok(edit) => Some(edit),
                Err(e) => {
                    self.error = Some(e);
                    cx.notify();
                    return;
                }
            }
        } else {
            None
        };
        let draft = EventDraft {
            title,
            when,
            time_zone: TimeZone::system().iana_name().map(str::to_string),
            location: Some(location),
            description: None,
            guests: vec![],
            meet: false,
            rrule: None,
            edit_guests,
            edit_reminders,
            series: (self.in_series && self.whole_series).then_some(SeriesEdit {
                change_time: matches!(when, When::Timed { .. }) && when != self.original_when,
            }),
        };
        let whole_series = self.in_series && self.whole_series;
        let (calendar_id, event_id) = (self.calendar_id.clone(), self.event_id.clone());
        let target_id = self.target_calendar_id.clone();
        let ok = self.store.update(cx, |store, cx| {
            let Some(calendar) = store.calendar(&calendar_id).cloned() else {
                return false;
            };
            if !store.can_write(&calendar) {
                return false;
            }
            let ok = if whole_series {
                store.update_series(&calendar, &event_id, draft)
            } else if target_id != calendar_id
                && let Some(target) = store.calendar(&target_id).cloned()
            {
                store.move_event(&calendar, &event_id, &target, Some(draft))
            } else {
                store.update_event(&calendar, &event_id, draft)
            };
            cx.notify();
            ok
        });
        if ok {
            cx.emit(ComposeEvent::Close);
        } else {
            self.error = Some(
                "This event can't be saved right now (editing is off, or it is still syncing)."
                    .into(),
            );
            cx.notify();
        }
    }

    fn delete(&mut self, cx: &mut Context<Self>) {
        if !self.confirm_delete {
            self.confirm_delete = true;
            cx.notify();
            return;
        }
        let whole_series = self.in_series && self.whole_series;
        let (calendar_id, event_id) = (self.calendar_id.clone(), self.event_id.clone());
        let ok = self.store.update(cx, |store, cx| {
            let Some(calendar) = store.calendar(&calendar_id).cloned() else {
                return false;
            };
            if !store.can_write(&calendar) {
                return false;
            }
            let ok = if whole_series {
                store.delete_series(&calendar, &event_id)
            } else {
                store.delete_event(&calendar, &event_id)
            };
            cx.notify();
            ok
        });
        if ok {
            cx.emit(ComposeEvent::Close);
        } else {
            self.error = Some("This event can't be deleted right now.".into());
            self.confirm_delete = false;
            cx.notify();
        }
    }
}

impl Focusable for EditorView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.title.focus_handle(cx)
    }
}

impl Render for EditorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let clock = store.prefs.clock_24h;
        let tz = TimeZone::system();
        let field = |label: &'static str, input: Entity<TextInput>| {
            div()
                .flex()
                .flex_col()
                .gap(px(4.0))
                .child(
                    div()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(MUTED))
                        .child(label),
                )
                .child(input)
        };

        let when_preview: Option<(String, u32)> = if self.when_locked {
            Some((
                "This event spans several days; change its time in Google Calendar.".into(),
                MUTED,
            ))
        } else {
            match self.parsed_when(cx) {
                Ok(when) => store.calendar(&self.calendar_id).map(|cal| {
                    let ev = Store::event_from_draft(
                        cal,
                        "preview".into(),
                        &EventDraft {
                            title: String::new(),
                            when,
                            time_zone: None,
                            location: None,
                            description: None,
                            guests: vec![],
                            meet: false,
                            rrule: None,
                            edit_guests: None,
                            edit_reminders: None,
                            series: None,
                        },
                    );
                    (format::event_when_long(&ev, &tz, clock), SECONDARY)
                }),
                Err(e) => Some((e, AMBER)),
            }
        };
        let calendar_label = store
            .calendar(&self.calendar_id)
            .map(|c| {
                if c.name == c.account {
                    c.name.clone()
                } else {
                    format!("{} · {}", c.name, c.account)
                }
            })
            .unwrap_or_default();
        let target_label = store
            .calendar(&self.target_calendar_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let can_pick_calendar = self.can_move && self.move_targets(cx).len() > 1;
        let error = self.error.clone();
        let confirm = self.confirm_delete;
        let whole_series_now = self.whole_series;

        backdrop()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(ComposeEvent::Close)),
            )
            .child(
                panel()
                    .on_action(cx.listener(|this, _: &Submit, _, cx| this.save(cx)))
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(SECONDARY))
                                    .child("Edit event"),
                            )
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(CAPTION))
                                    .child(SharedString::from(calendar_label)),
                            ),
                    )
                    .children(self.in_series.then(|| {
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(MUTED))
                                    .child("Changes apply to"),
                            )
                            .child(button(
                                "editor-scope",
                                if whole_series_now {
                                    "All events in the series ▸"
                                } else {
                                    "This event only ▸"
                                },
                                ButtonStyle::Secondary,
                                Some(Box::new(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.toggle_scope(cx)
                                }))),
                            ))
                    }))
                    .child(field("Title", self.title.clone()))
                    .child(field("When", self.when.clone()))
                    .children(when_preview.map(|(text, color)| {
                        div().text_size(px(12.0)).text_color(rgb(color)).child(text)
                    }))
                    .child(field("Location", self.location.clone()))
                    .children(can_pick_calendar.then(|| {
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .text_size(px(12.0))
                                    .text_color(rgb(MUTED))
                                    .child("Calendar"),
                            )
                            .child(button(
                                "editor-calendar",
                                SharedString::from(format!("{target_label} ▸")),
                                ButtonStyle::Secondary,
                                Some(Box::new(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.cycle_calendar(cx)
                                }))),
                            ))
                    }))
                    .child(if self.can_edit_guests {
                        field("Guests", self.guests.clone()).into_any_element()
                    } else {
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(MUTED))
                            .child("Only the organiser can change who is invited.")
                            .into_any_element()
                    })
                    .children(
                        self.can_edit_reminders
                            .then(|| field("Reminders", self.reminders.clone())),
                    )
                    .children(
                        error.map(|e| div().text_size(px(12.0)).text_color(rgb(AMBER)).child(e)),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(button(
                                "editor-delete",
                                match (confirm, whole_series_now) {
                                    (true, true) => "Click again to delete the series",
                                    (true, false) => "Click again to delete",
                                    (false, true) => "Delete series",
                                    (false, false) => "Delete",
                                },
                                ButtonStyle::Danger,
                                Some(Box::new(
                                    cx.listener(|this, _: &ClickEvent, _, cx| this.delete(cx)),
                                )),
                            ))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .child(button(
                                        "editor-cancel",
                                        "Cancel",
                                        ButtonStyle::Secondary,
                                        Some(Box::new(cx.listener(|_, _: &ClickEvent, _, cx| {
                                            cx.emit(ComposeEvent::Close)
                                        }))),
                                    ))
                                    .child(button(
                                        "editor-save",
                                        "Save",
                                        ButtonStyle::Primary,
                                        Some(Box::new(cx.listener(
                                            |this, _: &ClickEvent, _, cx| this.save(cx),
                                        ))),
                                    )),
                            ),
                    )
                    .child(hint("Enter to save · Tab for next field · Esc to cancel")),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn guests_are_read_as_a_list_of_emails() {
        assert_eq!(
            parse_guests("Ann@Example.com, bob@example.com; ann@example.com  cy@x.org"),
            Ok(vec![
                "ann@example.com".into(),
                "bob@example.com".into(),
                "cy@x.org".into()
            ])
        );
        assert_eq!(parse_guests("  "), Ok(vec![]));
        assert!(parse_guests("ann@example.com, bob").is_err());
        assert!(parse_guests("@example.com").is_err());
        assert!(parse_guests("a@b").is_err());
    }

    #[test]
    fn reminders_round_trip_through_text() {
        assert_eq!(
            reminders_to_text(&[60, 10, 1440, 10080, 90]),
            "10m, 1h, 90m, 1d, 1w"
        );
        assert_eq!(
            text_to_reminders("10m, 1h 1d, 2w"),
            Ok(RemindersEdit::Custom(vec![10, 60, 1440, 20160]))
        );
        assert_eq!(text_to_reminders("30"), Ok(RemindersEdit::Custom(vec![30])));
        assert_eq!(text_to_reminders("Default"), Ok(RemindersEdit::Defaults));
        assert_eq!(text_to_reminders("none"), Ok(RemindersEdit::Custom(vec![])));
        assert_eq!(text_to_reminders(""), Ok(RemindersEdit::Custom(vec![])));
        assert!(text_to_reminders("soon").is_err());
        assert!(text_to_reminders("10x").is_err());
        assert!(text_to_reminders("5w").is_err());
        let shown = reminders_to_text(&[5, 60, 2880]);
        assert_eq!(
            text_to_reminders(&shown),
            Ok(RemindersEdit::Custom(vec![5, 60, 2880]))
        );
    }

    fn now() -> Zoned {
        // No tz database in the Nix sandbox: build the zone from the constant.
        "2026-11-25T10:20:00Z"
            .parse::<jiff::Timestamp>()
            .unwrap()
            .to_zoned(TimeZone::UTC)
    }

    fn cal(account: &str, id: &str, name: &str, primary: bool) -> Calendar {
        Calendar {
            id: Calendar::key(account, id),
            account: account.into(),
            remote_id: id.into(),
            name: name.into(),
            color: 0x123456,
            primary,
            default_reminders: vec![10],
            selected_upstream: true,
            writable: true,
        }
    }

    fn cals() -> Vec<Calendar> {
        vec![
            cal("a@x.com", "a@x.com", "Personal", true),
            cal("a@x.com", "fam", "Family", false),
            cal("b@y.com", "b@y.com", "Work", true),
        ]
    }

    fn accounts() -> Vec<String> {
        vec!["a@x.com".into(), "b@y.com".into()]
    }

    #[test]
    fn default_calendar_is_the_primary_of_the_first_editable_account() {
        let c = cals();
        let only_b = |a: &str| a == "b@y.com";
        assert_eq!(
            pick_calendar(&c, &accounts(), None, only_b).unwrap().name,
            "Work"
        );
        let all = |_: &str| true;
        assert_eq!(
            pick_calendar(&c, &accounts(), None, all).unwrap().name,
            "Personal"
        );
        // No account can edit: still a calendar, so the UI can explain.
        let none = |_: &str| false;
        assert_eq!(
            pick_calendar(&c, &accounts(), None, none).unwrap().name,
            "Personal"
        );
    }

    #[test]
    fn read_only_calendars_are_not_picked_by_default_and_are_flagged_when_named() {
        let mut c = cals();
        // Personal (the primary) is read-only: the default moves to the account's writable one.
        c[0].writable = false;
        let all = |_: &str| true;
        assert_eq!(
            pick_calendar(&c, &accounts(), None, all).unwrap().name,
            "Family"
        );
        // Naming it anyway gets it, and resolve says why it cannot be used.
        let now = now();
        let parsed = quickadd::parse("Lunch tomorrow 1pm /personal", &now);
        let r = resolve(parsed, now.date(), &now, &c, &accounts(), all).unwrap();
        assert_eq!(r.calendar.name, "Personal");
        assert_eq!(r.problem, Some(Problem::ReadOnly("Personal".into())));
        // A name that matches a writable calendar wins over a read-only one.
        c[1].name = "Personal 2".into();
        let parsed = quickadd::parse("Lunch tomorrow 1pm /personal", &now);
        let r = resolve(parsed, now.date(), &now, &c, &accounts(), all).unwrap();
        assert_eq!(r.calendar.name, "Personal 2");
        assert_eq!(r.problem, None);
    }

    #[test]
    fn slash_hint_matches_names_loosely() {
        let c = cals();
        let all = |_: &str| true;
        assert_eq!(
            pick_calendar(&c, &accounts(), Some("fam"), all)
                .unwrap()
                .name,
            "Family"
        );
        assert_eq!(
            pick_calendar(&c, &accounts(), Some("WORK"), all)
                .unwrap()
                .name,
            "Work"
        );
        // Unknown hint falls back to the default rather than failing.
        assert_eq!(
            pick_calendar(&c, &accounts(), Some("nope"), all)
                .unwrap()
                .name,
            "Personal"
        );
    }

    #[test]
    fn resolve_builds_the_flagship_draft() {
        let c = cals();
        let now = now();
        let parsed = quickadd::parse("Lunch with Sam tomorrow 1pm at Dishoom /work", &now);
        let r = resolve(parsed, now.date(), &now, &c, &accounts(), |_| true).unwrap();
        assert_eq!(r.problem, None);
        assert_eq!(r.calendar.name, "Work");
        assert_eq!(r.draft.title, "Lunch with Sam");
        assert_eq!(r.draft.location.as_deref(), Some("Dishoom"));
        assert_eq!(r.draft.time_zone.as_deref(), Some("UTC"));
        let When::Timed { start, .. } = r.draft.when else {
            panic!()
        };
        assert_eq!(start.to_string(), "2026-11-26T13:00:00Z");
    }

    #[test]
    fn resolve_flags_problems_without_dropping_the_draft() {
        let c = cals();
        let now = now();
        let r = resolve(
            quickadd::parse("x", &now),
            now.date(),
            &now,
            &c,
            &accounts(),
            |_| false,
        )
        .unwrap();
        assert_eq!(r.problem, Some(Problem::EditingOff("a@x.com".into())));
        let r = resolve(
            quickadd::parse("tomorrow", &now),
            now.date(),
            &now,
            &c,
            &accounts(),
            |_| true,
        )
        .unwrap();
        assert_eq!(r.problem, Some(Problem::EmptyTitle));
        assert!(
            resolve(
                quickadd::parse("x", &now),
                now.date(),
                &now,
                &[],
                &[],
                |_| true
            )
            .is_none()
        );
    }

    #[test]
    fn untimed_text_defaults_to_the_next_hour_or_nine_on_another_day() {
        let now = now();
        let When::Timed { start, end } = default_when(now.date(), &now) else {
            panic!()
        };
        assert_eq!(start.to_string(), "2026-11-25T11:00:00Z");
        assert_eq!(end.to_string(), "2026-11-25T12:00:00Z");
        let When::Timed { start, .. } = default_when(date(2026, 12, 2), &now) else {
            panic!()
        };
        assert_eq!(start.to_string(), "2026-12-02T09:00:00Z");
    }

    #[test]
    fn the_editors_when_text_round_trips() {
        let tz = TimeZone::UTC;
        let now = now();
        let timed = When::Timed {
            start: "2026-11-27T13:00:00Z".parse().unwrap(),
            end: "2026-11-27T14:30:00Z".parse().unwrap(),
        };
        let text = when_to_text(&timed, &tz).unwrap();
        assert_eq!(text, "2026-11-27 13:00-14:30");
        assert_eq!(text_to_when(&text, &now), Ok(timed));

        let all_day = When::AllDay {
            start: date(2026, 12, 3),
            end: date(2026, 12, 4),
        };
        let text = when_to_text(&all_day, &tz).unwrap();
        assert_eq!(text_to_when(&text, &now), Ok(all_day));

        let overnight = When::Timed {
            start: "2026-11-27T22:00:00Z".parse().unwrap(),
            end: "2026-11-28T02:00:00Z".parse().unwrap(),
        };
        let text = when_to_text(&overnight, &tz).unwrap();
        assert_eq!(text_to_when(&text, &now), Ok(overnight));
    }

    #[test]
    fn unrepresentable_times_are_locked() {
        let tz = TimeZone::UTC;
        let multi = When::AllDay {
            start: date(2026, 12, 3),
            end: date(2026, 12, 6),
        };
        assert_eq!(when_to_text(&multi, &tz), None);
        let zero = When::Timed {
            start: "2026-11-27T13:00:00Z".parse().unwrap(),
            end: "2026-11-27T13:00:00Z".parse().unwrap(),
        };
        assert_eq!(when_to_text(&zero, &tz), None);
        let long = When::Timed {
            start: "2026-11-27T13:00:00Z".parse().unwrap(),
            end: "2026-11-29T13:00:00Z".parse().unwrap(),
        };
        assert_eq!(when_to_text(&long, &tz), None);
    }

    #[test]
    fn editor_rejects_text_it_cannot_read() {
        let now = now();
        assert!(text_to_when("whenever", &now).is_err());
        assert!(text_to_when("", &now).is_err());
        assert!(text_to_when("tomorrow 2-3pm", &now).is_ok());
    }
}
