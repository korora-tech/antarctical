//! The import overlay: what an `.ics` file holds, which calendar it goes to, and one button.

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    MouseButton, Window, div, px, rgb,
};
use jiff::tz::TimeZone;

use super::compose::{ComposeEvent, backdrop, fact, hint, panel};
use super::theme::{AMBER, MUTED, SECONDARY};
use super::widgets::{ButtonStyle, OnClick, button};
use crate::ics::Parsed;
use crate::model::Calendar;
use crate::store::Store;

/// How many events the overlay lists before saying "and N more".
const SHOWN: usize = 6;

pub struct ImportView {
    store: Entity<Store>,
    name: String,
    outcome: Result<Parsed, String>,
    calendar_id: Option<String>,
    focus: FocusHandle,
}

impl EventEmitter<ComposeEvent> for ImportView {}

impl ImportView {
    pub fn new(
        store: Entity<Store>,
        name: String,
        outcome: Result<Parsed, String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let calendar_id = {
            let s = store.read(cx);
            crate::ui::compose::pick_calendar(&s.calendars, &s.accounts, None, |a| s.can_edit(a))
                .map(|c| c.id.clone())
        };
        Self {
            store,
            name,
            outcome,
            calendar_id,
            focus: cx.focus_handle(),
        }
    }

    pub fn first_focus(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }

    /// The calendars an import can go to: writable ones whose account has editing on.
    fn targets(&self, cx: &App) -> Vec<Calendar> {
        let s = self.store.read(cx);
        s.calendars
            .iter()
            .filter(|c| s.can_write(c))
            .cloned()
            .collect()
    }

    fn cycle(&mut self, cx: &mut Context<Self>) {
        let targets = self.targets(cx);
        let next = targets
            .iter()
            .position(|c| Some(&c.id) == self.calendar_id.as_ref())
            .map_or(0, |i| (i + 1) % targets.len().max(1));
        self.calendar_id = targets.get(next).map(|c| c.id.clone());
        cx.notify();
    }

    fn import(&mut self, cx: &mut Context<Self>) {
        let Ok(parsed) = &self.outcome else { return };
        let Some(calendar) = self
            .calendar_id
            .as_deref()
            .and_then(|id| self.store.read(cx).calendar(id).cloned())
        else {
            return;
        };
        let drafts: Vec<_> = parsed.events.iter().map(|e| e.draft()).collect();
        let count = drafts.len();
        self.store.update(cx, |store, cx| {
            if !store.can_write(&calendar) {
                return;
            }
            for draft in drafts {
                store.create_event(&calendar, draft);
            }
            store.notice = Some((
                format!("Importing {count} events to {}", calendar.name),
                std::time::Instant::now(),
            ));
            cx.notify();
        });
        cx.emit(ComposeEvent::Close);
    }
}

impl Focusable for ImportView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ImportView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tz = TimeZone::system();
        let store = self.store.read(cx);
        let clock = store.prefs.clock_24h;
        let calendar = self
            .calendar_id
            .as_deref()
            .and_then(|id| store.calendar(id))
            .cloned();
        let writable_targets = self.targets(cx).len();
        let store = self.store.read(cx);
        let editing_off = calendar
            .as_ref()
            .is_some_and(|c| !store.can_edit(&c.account))
            || (calendar.is_none() && !store.calendars.is_empty());

        let blank = Calendar {
            id: String::new(),
            account: String::new(),
            remote_id: String::new(),
            name: String::new(),
            color: 0,
            primary: false,
            default_reminders: vec![],
            selected_upstream: true,
            writable: true,
        };
        let mut body = div().flex().flex_col().gap(px(6.0));
        let mut can_import = false;
        match &self.outcome {
            Err(message) => {
                body = body.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(AMBER))
                        .child(message.clone()),
                );
            }
            Ok(parsed) if parsed.events.is_empty() => {
                body = body.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(AMBER))
                        .child("No events found in this file."),
                );
            }
            Ok(parsed) => {
                for e in parsed.events.iter().take(SHOWN) {
                    let ev = Store::event_from_draft(
                        calendar.as_ref().unwrap_or(&blank),
                        "preview".into(),
                        &e.draft(),
                    );
                    body = body.child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(13.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(e.title.clone()),
                            )
                            .child(fact(
                                "When",
                                format!(
                                    "{}{}",
                                    super::format::event_when_long(&ev, &tz, clock),
                                    if e.rrule.is_some() { " · repeats" } else { "" }
                                ),
                                SECONDARY,
                            )),
                    );
                }
                if parsed.events.len() > SHOWN {
                    body = body.child(
                        div()
                            .text_size(px(12.0))
                            .text_color(rgb(MUTED))
                            .child(format!("and {} more", parsed.events.len() - SHOWN)),
                    );
                }
                if parsed.skipped > 0 {
                    body = body.child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(MUTED))
                            .child(format!(
                                "{} left out: cancelled, changed single occurrences, or no usable start time.",
                                parsed.skipped
                            )),
                    );
                }
                can_import = calendar.is_some() && !editing_off;
            }
        }
        let count = self.outcome.as_ref().map_or(0, |p| p.events.len());
        let target_label = calendar
            .as_ref()
            .map_or("no calendar".to_string(), |c| c.name.clone());

        backdrop()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|_, _, _, cx| cx.emit(ComposeEvent::Close)),
            )
            .child(
                panel()
                    .track_focus(&self.focus)
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(SECONDARY))
                            .child(format!("Import {}", self.name)),
                    )
                    .child(body)
                    .when(count > 0, |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_size(px(12.0))
                                        .text_color(rgb(MUTED))
                                        .child("Into"),
                                )
                                .child(button(
                                    "import-calendar",
                                    format!("{target_label} ▸"),
                                    ButtonStyle::Secondary,
                                    (writable_targets > 1).then(|| {
                                        Box::new(
                                            cx.listener(|this, _: &ClickEvent, _, cx| {
                                                this.cycle(cx)
                                            }),
                                        ) as OnClick
                                    }),
                                )),
                        )
                    })
                    .when(editing_off && count > 0, |d| {
                        d.child(div().text_size(px(12.0)).text_color(rgb(AMBER)).child(
                            "Editing is off for this account. Turn it on in Settings first.",
                        ))
                    })
                    .when(count > 1, |d| {
                        d.child(hint(
                            "Guests are not invited; events get your calendar's default reminders.",
                        ))
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(hint("Esc to cancel"))
                            .child(
                                div()
                                    .flex()
                                    .gap_2()
                                    .when(editing_off && count > 0, |d| {
                                        d.child(button(
                                            "import-settings",
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
                                        "import-cancel",
                                        "Cancel",
                                        ButtonStyle::Secondary,
                                        Some(Box::new(cx.listener(|_, _: &ClickEvent, _, cx| {
                                            cx.emit(ComposeEvent::Close)
                                        }))),
                                    ))
                                    .child(button(
                                        "import-go",
                                        if count == 1 {
                                            "Import 1 event".to_string()
                                        } else {
                                            format!("Import {count} events")
                                        },
                                        ButtonStyle::Primary,
                                        can_import.then(|| {
                                            Box::new(cx.listener(|this, _: &ClickEvent, _, cx| {
                                                this.import(cx)
                                            }))
                                                as OnClick
                                        }),
                                    )),
                            ),
                    ),
            )
    }
}
