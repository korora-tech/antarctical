//! `MainView`: header, sidebar, the current calendar view or Settings, and the event detail panel.

use std::time::Duration;

use gpui::{
    AnyElement, ClickEvent, Context, Div, Entity, FocusHandle, Focusable, FontWeight, ScrollHandle,
    SharedString, Subscription, Task, Window, div, prelude::*, px, rgb,
};
use jiff::Zoned;
use jiff::civil::Date;
use jiff::tz::TimeZone;

use super::format;
use super::settings::{SettingsEvent, SettingsView};
use super::theme::*;
use super::widgets::{
    ButtonStyle, OnClick, body_text, button, caption, card, checkbox_row, ghost_button, segment,
};
use super::{
    Back, CloseWindow, Next, OpenSettings, Previous, Quit, Refresh, ShowAgenda, ShowDay, ShowMonth,
    ShowWeek, Today,
};
use crate::config::ViewMode;
use crate::model::{Event, Response, add_days, add_months, month_grid, months_between, week_start};
use crate::store::Store;
use crate::sync::{AuthState, Command, ViewId};

const NOTICE_FOR: Duration = Duration::from_secs(4);
pub const AGENDA_DAYS: i32 = 30;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Calendar,
    Settings,
}

pub struct MainView {
    pub(super) store: Entity<Store>,
    settings: Entity<SettingsView>,
    screen: Screen,
    pub(super) view: ViewMode,
    pub(super) anchor: Date,
    /// (calendar id, event id) shown in the detail panel.
    pub(super) selected: Option<(String, String)>,
    focus: FocusHandle,
    pub(super) grid_scroll: ScrollHandle,
    pub(super) pending_grid_scroll: bool,
    pub(super) agenda_scroll: ScrollHandle,
    detail_scroll: ScrollHandle,
    wanted: Vec<Date>,
    _subscriptions: Vec<Subscription>,
    _ticker: Task<()>,
}

impl MainView {
    pub fn new(store: Entity<Store>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let settings = cx.new(|cx| SettingsView::new(store.clone(), window, cx));
        let subs = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &settings,
                window,
                |this, _, ev: &SettingsEvent, window, cx| match ev {
                    SettingsEvent::Close => this.show_calendar(window, cx),
                },
            ),
        ];
        // Keeps the now-line, "today" and toast expiry current.
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(15))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        let view = store.read(cx).prefs.view;
        let mut this = Self {
            store,
            settings,
            screen: Screen::Calendar,
            view,
            anchor: Zoned::now().date(),
            selected: None,
            focus: cx.focus_handle(),
            grid_scroll: ScrollHandle::new(),
            pending_grid_scroll: true,
            agenda_scroll: ScrollHandle::new(),
            detail_scroll: ScrollHandle::new(),
            wanted: vec![],
            _subscriptions: subs,
            _ticker: ticker,
        };
        this.request_range(cx);
        if !this.store.read(cx).signed_in()
            && matches!(this.store.read(cx).auth, AuthState::NeedsClient)
        {
            this.screen = Screen::Settings;
        }
        this
    }

    pub fn focus_calendar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.screen {
            Screen::Calendar => window.focus(&self.focus, cx),
            Screen::Settings => {
                let handle = self.settings.read(cx).first_focus(cx);
                window.focus(&handle, cx)
            }
        }
    }

    pub fn focus_date(&mut self, date: Date, window: &mut Window, cx: &mut Context<Self>) {
        self.anchor = date;
        self.selected = None;
        self.pending_grid_scroll = true;
        self.agenda_scroll.set_offset(gpui::point(px(0.0), px(0.0)));
        self.request_range(cx);
        self.focus_calendar(window, cx);
        cx.notify();
    }

    // ----- navigation ---------------------------------------------------------------------

    /// The half-open date range the current view shows.
    pub(super) fn visible_range(&self) -> (Date, Date) {
        match self.view {
            ViewMode::Month => {
                let grid = month_grid(self.anchor);
                (grid[0], add_days(grid[41], 1))
            }
            ViewMode::Week => {
                let s = week_start(self.anchor);
                (s, add_days(s, 7))
            }
            ViewMode::Day => (self.anchor, add_days(self.anchor, 1)),
            ViewMode::Agenda => (self.anchor, add_days(self.anchor, AGENDA_DAYS)),
        }
    }

    fn request_range(&mut self, cx: &mut Context<Self>) {
        let (from, to) = self.visible_range();
        let months = months_between(from, to);
        if months != self.wanted {
            self.wanted = months.clone();
            self.store
                .read(cx)
                .send(Command::Want(ViewId::Main, months));
        }
    }

    fn set_anchor(&mut self, anchor: Date, cx: &mut Context<Self>) {
        self.anchor = anchor;
        self.request_range(cx);
        cx.notify();
    }

    fn set_view(&mut self, view: ViewMode, cx: &mut Context<Self>) {
        if self.view != view {
            self.view = view;
            self.pending_grid_scroll = true;
            self.store.update(cx, |s, _| {
                s.prefs.view = view;
                s.prefs.save();
            });
        }
        self.request_range(cx);
        cx.notify();
    }

    fn step(&mut self, dir: i32, cx: &mut Context<Self>) {
        let anchor = match self.view {
            ViewMode::Month => add_months(self.anchor.first_of_month(), dir),
            ViewMode::Week => add_days(self.anchor, 7 * dir),
            ViewMode::Day => add_days(self.anchor, dir),
            ViewMode::Agenda => add_days(self.anchor, AGENDA_DAYS * dir),
        };
        self.set_anchor(anchor, cx);
    }

    pub(super) fn open_day(&mut self, date: Date, cx: &mut Context<Self>) {
        self.anchor = date;
        self.pending_grid_scroll = true;
        self.set_view(ViewMode::Day, cx);
    }

    pub(super) fn select(&mut self, e: &Event, cx: &mut Context<Self>) {
        self.selected = Some((e.calendar_id.clone(), e.id.clone()));
        self.detail_scroll.set_offset(gpui::point(px(0.), px(0.)));
        cx.notify();
    }

    pub(super) fn is_selected(&self, e: &Event) -> bool {
        self.selected
            .as_ref()
            .is_some_and(|(c, i)| *c == e.calendar_id && *i == e.id)
    }

    fn show_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.screen = Screen::Settings;
        self.focus_calendar(window, cx);
        cx.notify();
    }

    fn show_calendar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.screen = Screen::Calendar;
        self.pending_grid_scroll = true;
        window.focus(&self.focus, cx);
        cx.notify();
    }

    fn back(&mut self, _: &Back, window: &mut Window, cx: &mut Context<Self>) {
        match self.screen {
            Screen::Settings => self.show_calendar(window, cx),
            Screen::Calendar => {
                self.selected = None;
                cx.notify();
            }
        }
    }

    pub(super) fn listener_click(
        &self,
        cx: &Context<Self>,
        f: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> OnClick {
        Box::new(cx.listener(move |this, _: &ClickEvent, window, cx| f(this, window, cx)))
    }

    // ----- chrome -------------------------------------------------------------------------

    fn header(&self, cx: &Context<Self>) -> Div {
        let in_calendar = self.screen == Screen::Calendar;
        let view_tabs = [
            ("seg-month", "Month", ViewMode::Month),
            ("seg-week", "Week", ViewMode::Week),
            ("seg-day", "Day", ViewMode::Day),
            ("seg-agenda", "Agenda", ViewMode::Agenda),
        ];
        div()
            .h(px(64.0))
            .w_full()
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .gap_3()
            .px(px(22.0))
            .bg(rgb(HEADER))
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .min_w(px(0.0))
                    .child(
                        div()
                            .size(px(32.0))
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .rounded_md()
                            .bg(rgb(ACCENT))
                            .text_size(px(13.0))
                            .font_weight(FontWeight::BOLD)
                            .text_color(rgb(0x171323))
                            .child(Zoned::now().day().to_string()),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_size(px(14.0))
                            .text_color(rgb(TEXT))
                            .child("antarctical"),
                    )
                    .when(in_calendar, |d| {
                        d.child(div().w(px(1.0)).h(px(26.0)).bg(rgb(BORDER)))
                            .child(
                                div()
                                    .text_size(px(19.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(TEXT))
                                    .child(format::view_title(self.view, self.anchor)),
                            )
                            .child(ghost_button(
                                "prev",
                                "‹",
                                Some(self.listener_click(cx, |this, _, cx| this.step(-1, cx))),
                            ))
                            .child(ghost_button(
                                "next",
                                "›",
                                Some(self.listener_click(cx, |this, _, cx| this.step(1, cx))),
                            ))
                            .child(button(
                                "today",
                                "Today",
                                ButtonStyle::Secondary,
                                Some(self.listener_click(cx, |this, _, cx| {
                                    this.set_anchor(Zoned::now().date(), cx);
                                    this.pending_grid_scroll = true;
                                })),
                            ))
                    }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(in_calendar, |d| {
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .rounded_lg()
                                .border_1()
                                .border_color(rgb(BORDER))
                                .bg(rgb(BG))
                                .p(px(3.0))
                                .children(view_tabs.into_iter().map(|(id, label, mode)| {
                                    segment(
                                        id,
                                        label,
                                        self.view == mode,
                                        self.listener_click(cx, move |this, _, cx| {
                                            this.set_view(mode, cx)
                                        }),
                                    )
                                })),
                        )
                    })
                    .child(button(
                        "settings",
                        if in_calendar {
                            "Settings"
                        } else {
                            "Back to calendar"
                        },
                        if in_calendar {
                            ButtonStyle::Secondary
                        } else {
                            ButtonStyle::Primary
                        },
                        Some(self.listener_click(cx, |this, window, cx| {
                            if this.screen == Screen::Settings {
                                this.show_calendar(window, cx)
                            } else {
                                this.show_settings(window, cx)
                            }
                        })),
                    )),
            )
    }

    fn banner(&self, store: &Store, cx: &Context<Self>) -> Option<Div> {
        let problem = store.problem.clone()?;
        Some(
            div()
                .flex()
                .items_center()
                .gap_2()
                .px_5()
                .py_2()
                .bg(rgb(0x2a1a12))
                .border_b_1()
                .border_color(rgb(0x5a3a1a))
                .child(
                    div()
                        .size(px(6.0))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(rgb(AMBER)),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .text_size(px(12.0))
                        .text_color(rgb(0xffd9a0))
                        .child(problem),
                )
                .child(ghost_button(
                    "dismiss-problem",
                    "✕",
                    Some(self.listener_click(cx, |this, _, cx| {
                        this.store.read(cx).send(Command::DismissProblems)
                    })),
                )),
        )
    }

    fn sidebar(&self, store: &Store, cx: &Context<Self>) -> Div {
        let today = Zoned::now().date();
        let month = self.anchor.first_of_month();
        let mini = month_grid(month);
        let weekdays = ["M", "T", "W", "T", "F", "S", "S"];
        let (range_from, range_to) = self.visible_range();
        let mut grid = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .child(div().flex().children(weekdays.iter().map(|d| {
                div()
                    .flex_1()
                    .flex()
                    .justify_center()
                    .text_size(px(10.0))
                    .text_color(rgb(CAPTION))
                    .child(*d)
            })));
        for week in mini.chunks(7) {
            grid = grid.child(div().flex().children(week.iter().map(|&d| {
                let in_month = d.month() == month.month();
                let is_today = d == today;
                let in_view = self.screen == Screen::Calendar
                    && self.view != ViewMode::Month
                    && d >= range_from
                    && d < range_to;
                div()
                    .id(SharedString::from(format!("mini-{d}")))
                    .flex_1()
                    .h(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_md()
                    .text_size(px(11.0))
                    .when(in_view, |s| s.bg(rgb(SELECTED)))
                    .when(is_today, |s| {
                        s.bg(rgb(ACCENT))
                            .text_color(rgb(0x11111a))
                            .font_weight(FontWeight::BOLD)
                    })
                    .when(!is_today, |s| {
                        s.text_color(rgb(if in_month { SECONDARY } else { DISABLED }))
                    })
                    .hover(|s| s.bg(rgb(HOVER)).cursor_pointer())
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        if this.screen == Screen::Settings {
                            this.show_calendar(window, cx);
                        }
                        this.pending_grid_scroll = true;
                        if this.view == ViewMode::Month {
                            this.open_day(d, cx);
                        } else {
                            this.set_anchor(d, cx);
                        }
                    }))
                    .child(d.day().to_string())
            })));
        }

        // One group per account, headed by its email.
        let calendars =
            div()
                .flex()
                .flex_col()
                .gap_3()
                .children(
                    store
                        .calendars_by_account()
                        .into_iter()
                        .map(|(account, cals)| {
                            div()
                                .flex()
                                .flex_col()
                                .gap(px(1.0))
                                .child(div().pb_1().child(caption(account.to_uppercase())))
                                .children(cals.into_iter().map(|c| {
                                    let id = c.id.clone();
                                    checkbox_row(
                                        SharedString::from(format!("cal-{}", c.id)),
                                        // The primary calendar is named after its account,
                                        // which already heads the group.
                                        if c.primary {
                                            "Primary".to_string()
                                        } else {
                                            c.name.clone()
                                        },
                                        c.color,
                                        store.is_visible(c),
                                        self.listener_click(cx, move |this, _, cx| {
                                            let id = id.clone();
                                            this.store.update(cx, |s, cx| {
                                                s.toggle_calendar(&id);
                                                cx.notify();
                                            });
                                        }),
                                    )
                                }))
                        }),
                );

        div()
            .w(px(SIDEBAR_W))
            .flex_shrink_0()
            .h_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_4()
            .bg(rgb(SIDEBAR))
            .border_r_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(12.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT))
                            .child(month.strftime("%B %Y").to_string()),
                    )
                    .child(
                        div()
                            .flex()
                            .child(ghost_button(
                                "mini-prev",
                                "‹",
                                Some(self.listener_click(cx, move |this, _, cx| {
                                    this.set_anchor(add_months(month, -1), cx)
                                })),
                            ))
                            .child(ghost_button(
                                "mini-next",
                                "›",
                                Some(self.listener_click(cx, move |this, _, cx| {
                                    this.set_anchor(add_months(month, 1), cx)
                                })),
                            )),
                    ),
            )
            .child(grid)
            .child(
                div()
                    .id("calendar-list")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .when(!store.calendars.is_empty(), |d| d.child(calendars)),
            )
            .child(self.account_box(store, cx))
    }

    fn account_box(&self, store: &Store, cx: &Context<Self>) -> Div {
        let (dot, line1) = match (&store.auth, store.accounts.as_slice()) {
            (AuthState::Starting, _) => (DISABLED, "Opening keyring…".to_string()),
            (AuthState::NeedsClient, _) => (AMBER, "Google client not set up".into()),
            (AuthState::Locked, _) => (AMBER, "Keyring locked".into()),
            (AuthState::Demo, _) => (ACCENT, "Demo data".into()),
            _ if store.signing_in.is_some() => (AMBER, "Waiting for browser…".into()),
            (AuthState::Ready, []) => (AMBER, "Not signed in".into()),
            (AuthState::Ready, [one]) => (GREEN, one.clone()),
            (AuthState::Ready, many) => (GREEN, format!("{} Google accounts", many.len())),
        };
        let clock = store.prefs.clock_24h;
        let line2 = if store.busy {
            "Syncing…".to_string()
        } else if let Some(t) = store.last_sync {
            format!(
                "Synced {}",
                format::time(&t.to_zoned(TimeZone::system()), clock)
            )
        } else {
            String::new()
        };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .pt_3()
            .border_t_1()
            .border_color(rgb(BORDER))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .size(px(7.0))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(rgb(dot)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(11.0))
                            .text_color(rgb(SECONDARY))
                            .child(line1),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(10.0))
                            .text_color(rgb(MUTED))
                            .child(line2),
                    )
                    .when(store.signed_in(), |d| {
                        d.child(ghost_button(
                            "refresh",
                            "Refresh",
                            (!store.busy).then(|| {
                                self.listener_click(cx, |this, _, cx| {
                                    this.store.read(cx).send(Command::Refresh)
                                })
                            }),
                        ))
                    }),
            )
    }

    /// What to show instead of a calendar when there is no data source yet.
    fn auth_card(&self, store: &Store, cx: &Context<Self>) -> Option<AnyElement> {
        let content = match &store.auth {
            AuthState::Demo => return None,
            AuthState::Ready if store.signed_in() => return None,
            AuthState::Locked => card("Unlock your keyring")
                .child(body_text("Antarctical could not read your Google credentials from the system keyring. Unlock it (your desktop may have shown a prompt), then retry."))
                .child(div().flex().child(button(
                    "retry-keyring",
                    "Retry",
                    ButtonStyle::Primary,
                    Some(self.listener_click(cx, |this, _, cx| this.store.read(cx).send(Command::Refresh))),
                ))),
            AuthState::Starting => card("Opening your keyring…")
                .child(body_text("Antarctical keeps your Google credentials in the system keyring. Your desktop may ask you to unlock it.")),
            AuthState::NeedsClient => card("Connect Google Calendar")
                .child(body_text(
                    "Antarctical talks to Google with your own OAuth client, so no third party sits between you and your calendar. Add its client ID and secret in Settings; they are stored in your system keyring.",
                ))
                .child(div().flex().child(button(
                    "go-settings",
                    "Open Settings",
                    ButtonStyle::Primary,
                    Some(self.listener_click(cx, |this, window, cx| this.show_settings(window, cx))),
                ))),
            AuthState::Ready => match store.signing_in.clone() {
                None => card("Sign in to Google")
                    .child(body_text("Your browser will open Google's consent page. Antarctical only asks for read-only access to your calendars. You can add more accounts later in Settings."))
                    .child(div().flex().child(button(
                        "sign-in",
                        "Sign in with Google",
                        ButtonStyle::Primary,
                        Some(self.listener_click(cx, |this, _, cx| this.store.read(cx).send(Command::AddAccount))),
                    ))),
                Some(url) => card("Finish signing in in your browser")
                    .child(body_text("Approve access on Google's page. If no browser opened, copy the link and open it yourself."))
                    .child(sign_in_actions(url, |this: &mut Self, cx| this.store.read(cx).send(Command::CancelSignIn), self, cx)),
            },
        };
        Some(
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p_8()
                .child(div().w(px(460.0)).child(content))
                .into_any_element(),
        )
    }

    fn detail_panel(&self, store: &Store, cx: &Context<Self>) -> Option<Div> {
        let (cal_id, ev_id) = self.selected.as_ref()?;
        let tz = TimeZone::system();
        let (from, to) = self.visible_range();
        let event = store
            .events_between(add_days(from, -31), add_days(to, 31), &tz)
            .into_iter()
            .find(|e| e.calendar_id == *cal_id && e.id == *ev_id)?;
        let color = store.color_of(event);
        let cal_name = store
            .calendar(&event.calendar_id)
            .map(|c| {
                if store.accounts.len() > 1 && c.name != c.account {
                    format!("{} · {}", c.name, c.account)
                } else {
                    c.name.clone()
                }
            })
            .unwrap_or_default();
        let clock = store.prefs.clock_24h;

        let row = |label: &'static str, value: String| {
            div()
                .flex()
                .flex_col()
                .gap(px(2.0))
                .child(caption(label))
                .child(
                    div()
                        .text_size(px(12.0))
                        .line_height(px(18.0))
                        .text_color(rgb(SECONDARY))
                        .child(value),
                )
        };

        let mut body = div()
            .flex()
            .flex_col()
            .gap_4()
            .child(row("WHEN", format::event_when_long(event, &tz, clock)))
            .child(row("CALENDAR", cal_name));
        if let Some(loc) = &event.location {
            body = body.child(row("WHERE", loc.clone()));
        }
        if let Some(resp) = event.self_response() {
            let text = match resp {
                Response::Accepted => "You're going",
                Response::Tentative => "You might go",
                Response::Declined => "You declined",
                Response::NeedsAction => "You haven't replied",
            };
            body = body.child(row("YOUR RESPONSE", text.into()));
        }
        if let Some(org) = &event.organizer {
            body = body.child(row("ORGANIZER", org.clone()));
        }
        if !event.attendees.is_empty() {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(caption(format!("GUESTS · {}", event.attendees.len())))
                    .children(event.attendees.iter().take(50).map(|a| {
                        let (mark, mark_color) = match a.response {
                            Response::Accepted => ("✓", GREEN),
                            Response::Declined => ("✕", RED),
                            Response::Tentative => ("?", AMBER),
                            Response::NeedsAction => ("·", MUTED),
                        };
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(12.0))
                                    .flex_shrink_0()
                                    .text_size(px(11.0))
                                    .text_color(rgb(mark_color))
                                    .child(mark),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .truncate()
                                    .text_size(px(12.0))
                                    .text_color(rgb(SECONDARY))
                                    .child(format!(
                                        "{}{}",
                                        a.name,
                                        if a.organizer { " (organizer)" } else { "" }
                                    )),
                            )
                    })),
            );
        }
        if let Some(desc) = &event.description {
            body = body.child(row("DESCRIPTION", desc.clone()));
        }

        let mut actions = div().flex().flex_wrap().gap_2();
        if let Some(link) = event.meet_link.clone() {
            actions = actions.child(button(
                "join",
                "Join video call",
                ButtonStyle::Primary,
                Some(Box::new(move |_, _, cx| cx.open_url(&link))),
            ));
        }
        if let Some(link) = event.html_link.clone() {
            actions = actions.child(button(
                "open-google",
                "Open in Google Calendar",
                ButtonStyle::Secondary,
                Some(Box::new(move |_, _, cx| cx.open_url(&link))),
            ));
        }

        Some(
            div()
                .w(px(DETAIL_W))
                .flex_shrink_0()
                .h_full()
                .flex()
                .flex_col()
                .bg(rgb(PANEL))
                .border_l_1()
                .border_color(rgb(BORDER))
                .child(
                    div()
                        .flex()
                        .items_start()
                        .gap_3()
                        .p_4()
                        .border_b_1()
                        .border_color(rgb(BORDER))
                        .child(
                            div()
                                .mt(px(4.0))
                                .size(px(12.0))
                                .flex_shrink_0()
                                .rounded_sm()
                                .bg(rgb(color)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .text_size(px(16.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(TEXT))
                                .when(event.declined(), |d| d.line_through())
                                .child(event.title.clone()),
                        )
                        .child(ghost_button(
                            "close-detail",
                            "✕",
                            Some(self.listener_click(cx, |this, _, cx| {
                                this.selected = None;
                                cx.notify();
                            })),
                        )),
                )
                .child(
                    div()
                        .id("detail-scroll")
                        .flex_1()
                        .min_h(px(0.0))
                        .overflow_y_scroll()
                        .track_scroll(&self.detail_scroll)
                        .p_4()
                        .flex()
                        .flex_col()
                        .gap_4()
                        .child(actions)
                        .child(body),
                ),
        )
    }

    fn toast(&self, store: &Store) -> Option<Div> {
        let (msg, at) = store.notice.as_ref()?;
        if at.elapsed() > NOTICE_FOR {
            return None;
        }
        Some(
            div().absolute().bottom(px(18.0)).right(px(18.0)).child(
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(SELECTED_BORDER))
                    .bg(rgb(PANEL_RAISED))
                    .px_4()
                    .py_2()
                    .text_size(px(12.0))
                    .text_color(rgb(TEXT))
                    .child(msg.clone()),
            ),
        )
    }
}

impl Focusable for MainView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for MainView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A notice is shown for NOTICE_FOR; make sure the frame after that is drawn.
        if let Some((_, at)) = &self.store.read(cx).notice {
            let remaining = NOTICE_FOR.saturating_sub(at.elapsed());
            if !remaining.is_zero() {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(remaining).await;
                    let _ = this.update(cx, |_, cx| cx.notify());
                })
                .detach();
            }
        }
        let store_entity = self.store.clone();
        let store = store_entity.read(cx);
        let header = self.header(cx);
        let banner = self.banner(store, cx);
        let sidebar = self.sidebar(store, cx);
        let toast = self.toast(store);

        let main: AnyElement = match self.screen {
            Screen::Settings => self.settings.clone().into_any_element(),
            Screen::Calendar => match self.auth_card(store, cx) {
                Some(card) => card,
                None => {
                    let content = match self.view {
                        ViewMode::Month => self.month_view(store, window, cx),
                        ViewMode::Week => self.time_grid(store, week_start(self.anchor), 7, cx),
                        ViewMode::Day => self.time_grid(store, self.anchor, 1, cx),
                        ViewMode::Agenda => self.agenda_view(store, cx),
                    };
                    div()
                        .size_full()
                        .flex()
                        .child(div().flex_1().min_w(px(0.0)).h_full().child(content))
                        .children(self.detail_panel(store, cx))
                        .into_any_element()
                }
            },
        };

        div()
            .key_context("Antarctical")
            .on_action(cx.listener(Self::back))
            .on_action(
                cx.listener(|this, _: &OpenSettings, window, cx| this.show_settings(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &Refresh, _, cx| this.store.read(cx).send(Command::Refresh)),
            )
            .on_action(cx.listener(|_, _: &CloseWindow, window, _| window.remove_window()))
            .on_action(cx.listener(|_, _: &Quit, _, cx| cx.quit()))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .child(header)
            .children(banner)
            .child(
                div().flex_1().min_h(px(0.0)).flex().child(sidebar).child(
                    div()
                        // Only around the calendar: single-letter bindings must never
                        // apply while typing in Settings.
                        .when(self.screen == Screen::Calendar, |d| {
                            d.key_context("Calendar").track_focus(&self.focus)
                        })
                        .on_action(cx.listener(|this, _: &Today, _, cx| {
                            this.pending_grid_scroll = true;
                            this.set_anchor(Zoned::now().date(), cx)
                        }))
                        .on_action(cx.listener(|this, _: &Previous, _, cx| this.step(-1, cx)))
                        .on_action(cx.listener(|this, _: &Next, _, cx| this.step(1, cx)))
                        .on_action(cx.listener(|this, _: &ShowMonth, _, cx| {
                            this.set_view(ViewMode::Month, cx)
                        }))
                        .on_action(cx.listener(|this, _: &ShowWeek, _, cx| {
                            this.set_view(ViewMode::Week, cx)
                        }))
                        .on_action(
                            cx.listener(|this, _: &ShowDay, _, cx| {
                                this.set_view(ViewMode::Day, cx)
                            }),
                        )
                        .on_action(cx.listener(|this, _: &ShowAgenda, _, cx| {
                            this.set_view(ViewMode::Agenda, cx)
                        }))
                        .flex_1()
                        .min_w(px(0.0))
                        .h_full()
                        .overflow_hidden()
                        .child(main),
                ),
            )
            .children(toast)
    }
}

/// "Copy sign-in link" + "Cancel", shown while a browser sign-in is pending.
fn sign_in_actions(
    url: String,
    cancel: impl Fn(&mut MainView, &mut Context<MainView>) + 'static,
    view: &MainView,
    cx: &Context<MainView>,
) -> Div {
    div()
        .flex()
        .gap_2()
        .child(button(
            "copy-link",
            "Copy sign-in link",
            ButtonStyle::Secondary,
            Some(view.listener_click(cx, move |_, _, cx| {
                cx.write_to_clipboard(gpui::ClipboardItem::new_string(url.clone()))
            })),
        ))
        .child(button(
            "cancel-sign-in",
            "Cancel",
            ButtonStyle::Secondary,
            Some(view.listener_click(cx, move |this, _, cx| cancel(this, cx))),
        ))
}
