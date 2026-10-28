//! The compact, tray-triggered calendar window.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

#[cfg(target_os = "linux")]
use gpui::layer_shell::{Anchor, KeyboardInteractivity, Layer, LayerShellOptions};
use gpui::{
    App, AppContext, Bounds, Context, Entity, FocusHandle, FontWeight, PlatformDisplay, Render,
    ScrollHandle, SharedString, Stateful, Subscription, Task, Window, WindowBackgroundAppearance,
    WindowBounds, WindowDecorations, WindowHandle, WindowKind, div, point, prelude::*, px, rgb,
    size,
};
use jiff::Zoned;
use jiff::civil::Date;
use jiff::tz::TimeZone;

use super::format;
use super::theme::*;
use super::widgets::{ButtonStyle, button, ghost_button};
use super::{Back, show_main_window_at};
use crate::branding;
use crate::model::{add_days, months_between, week_start};
use crate::store::Store;
use crate::sync::{Command, ViewId};

const POPOVER_WIDTH: f32 = 368.0;
const POPOVER_HEIGHT: f32 = 640.0;

fn week_title(week: Date) -> String {
    let last_day = add_days(week, 6);
    if week.year() == last_day.year() && week.month() == last_day.month() {
        format!(
            "{}–{} {}",
            week.day(),
            last_day.day(),
            last_day.strftime("%B %Y")
        )
    } else if week.year() == last_day.year() {
        format!(
            "{} – {} {}",
            week.strftime("%-d %b"),
            last_day.strftime("%-d %b"),
            last_day.strftime("%Y")
        )
    } else {
        format!(
            "{} – {}",
            week.strftime("%-d %b %Y"),
            last_day.strftime("%-d %b %Y")
        )
    }
}

pub type TrayWindow = Rc<RefCell<Option<WindowHandle<TrayPopoverView>>>>;

pub struct TrayPopoverView {
    store: Entity<Store>,
    week: Date,
    selected: Date,
    focus: FocusHandle,
    agenda_scroll: ScrollHandle,
    _subscription: Subscription,
    _ticker: Task<()>,
}

impl TrayPopoverView {
    fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let today = Zoned::now().date();
        let subscription = cx.observe(&store, |_, _, cx| cx.notify());
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(20))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
        });
        let view = Self {
            store,
            week: week_start(today),
            selected: today,
            focus: cx.focus_handle(),
            agenda_scroll: ScrollHandle::new(),
            _subscription: subscription,
            _ticker: ticker,
        };
        view.request_week(cx);
        view
    }

    fn request_week(&self, cx: &Context<Self>) {
        self.store.read(cx).send(Command::Want(
            ViewId::Tray,
            months_between(self.week, add_days(self.week, 7)),
        ));
    }

    fn move_week(&mut self, delta: i32, cx: &mut Context<Self>) {
        let weekday_offset = self.selected.weekday().to_monday_zero_offset() as i32;
        self.week = add_days(self.week, delta * 7);
        self.selected = add_days(self.week, weekday_offset);
        self.agenda_scroll.set_offset(point(px(0.0), px(0.0)));
        self.request_week(cx);
        cx.notify();
    }

    fn select_date(&mut self, date: Date, cx: &mut Context<Self>) {
        self.week = week_start(date);
        self.selected = date;
        self.agenda_scroll.set_offset(point(px(0.0), px(0.0)));
        self.request_week(cx);
        cx.notify();
    }

    fn open_full(&self, date: Date, window: &mut Window, cx: &mut Context<Self>) {
        show_main_window_at(&self.store, date, cx);
        window.remove_window();
    }

    fn week_strip(&self, store: &Store, cx: &Context<Self>) -> gpui::Div {
        let today = Zoned::now().date();
        let dates: Vec<_> = (0..7).map(|offset| add_days(self.week, offset)).collect();
        let tz = TimeZone::system();
        let events = store.events_between(self.week, add_days(self.week, 7), &tz);

        div()
            .flex()
            .gap(px(2.0))
            .children(dates.into_iter().map(|date| {
                let is_today = date == today;
                let selected = date == self.selected;
                let colors: Vec<u32> = events
                    .iter()
                    .filter(|event| event.occurs_on(date, &tz))
                    .take(3)
                    .map(|event| store.color_of(event))
                    .collect();
                div()
                    .id(SharedString::from(format!("tray-day-{date}")))
                    .flex_1()
                    .min_w(px(0.0))
                    .h(px(62.0))
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(3.0))
                    .rounded_md()
                    .when(selected, |d| d.bg(rgb(SELECTED)))
                    .hover(|s| s.bg(rgb(HOVER)).cursor_pointer())
                    .on_click(cx.listener(move |this, _, _, cx| this.select_date(date, cx)))
                    .child(
                        div()
                            .text_size(px(9.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(if selected { ACCENT_TEXT } else { CAPTION }))
                            .child(date.strftime("%a").to_string()),
                    )
                    .child(
                        div()
                            .size(px(25.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .text_size(px(11.0))
                            .font_weight(if is_today || selected {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::MEDIUM
                            })
                            .text_color(rgb(if is_today || selected {
                                0x171323
                            } else {
                                SECONDARY
                            }))
                            .when(selected, |d| d.bg(rgb(ACCENT)))
                            .when(!selected && is_today, |d| {
                                d.border_1().border_color(rgb(ACCENT))
                            })
                            .child(date.day().to_string()),
                    )
                    .child(
                        div()
                            .h(px(4.0))
                            .flex()
                            .items_center()
                            .gap(px(2.0))
                            .children(
                                colors
                                    .into_iter()
                                    .map(|color| div().size(px(3.0)).rounded_full().bg(rgb(color))),
                            ),
                    )
            }))
    }

    fn day_section(
        &self,
        store: &Store,
        date: Date,
        today: Date,
        tz: &TimeZone,
        cx: &Context<Self>,
    ) -> gpui::Div {
        let title = if date == today {
            "TODAY".to_string()
        } else if date == add_days(today, 1) {
            "TOMORROW".to_string()
        } else {
            date.strftime("%A").to_string().to_uppercase()
        };
        let date_label = date.strftime("%-d %B").to_string();
        let events = store.events_on(date, tz);
        let event_count = match events.len() {
            0 => "NO EVENTS".to_string(),
            1 => "1 EVENT".to_string(),
            count => format!("{count} EVENTS"),
        };

        let mut section = div().flex().flex_col().gap(px(7.0)).child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(10.0))
                                .font_weight(FontWeight::BOLD)
                                .text_color(rgb(if date == today {
                                    ACCENT_TEXT
                                } else {
                                    SECONDARY
                                }))
                                .child(title),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .text_color(rgb(MUTED))
                                .child(date_label),
                        ),
                )
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(rgb(MUTED))
                        .child(event_count),
                ),
        );

        if events.is_empty() {
            section = section.child(
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(BG))
                    .px_3()
                    .py_2()
                    .text_size(px(11.0))
                    .text_color(rgb(MUTED))
                    .child("Nothing planned"),
            );
        } else {
            section = section.children(events.iter().copied().map(|event| {
                let color = store.color_of(event);
                let time = format::event_time_on(event, date, tz, store.prefs.clock_24h);
                let location = event.location.clone();
                let declined = event.declined();
                let event_id = format!("tray-event-{}-{}", event.calendar_id, event.id);
                let title = event.title.clone();
                div()
                    .id(SharedString::from(event_id))
                    .w_full()
                    .flex()
                    .items_start()
                    .gap_2()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(BG))
                    .px_2()
                    .py(px(8.0))
                    .hover(|s| s.bg(rgb(HOVER)).cursor_pointer())
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.open_full(date, window, cx)),
                    )
                    .child(
                        div()
                            .w(px(3.0))
                            .h(px(28.0))
                            .flex_shrink_0()
                            .rounded_full()
                            .bg(rgb(color)),
                    )
                    .child(
                        div()
                            .w(px(66.0))
                            .flex_shrink_0()
                            .pt(px(1.0))
                            .text_size(px(10.0))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(SECONDARY))
                            .child(time),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .flex_col()
                            .gap(px(3.0))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(rgb(if declined { MUTED } else { TEXT }))
                                    .when(declined, |d| d.line_through())
                                    .child(title),
                            )
                            .when_some(location, |d, location| {
                                d.child(
                                    div()
                                        .truncate()
                                        .text_size(px(10.0))
                                        .text_color(rgb(MUTED))
                                        .child(location),
                                )
                            }),
                    )
            }));
        }
        section
    }

    fn agenda(&self, store: &Store, cx: &Context<Self>) -> Stateful<gpui::Div> {
        let today = Zoned::now().date();
        let tz = TimeZone::system();
        let range_start = self.selected;
        let range_end = add_days(range_start, 2);
        let has_loaded_range = !store.events_between(range_start, range_end, &tz).is_empty();
        let no_calendars = store.calendars.is_empty();
        let mut body = div().flex().flex_col().gap_4().px_4().py_3();

        if no_calendars && !store.signed_in() {
            body = body.child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap_2()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(BG))
                    .p_4()
                    .child(
                        div()
                            .text_size(px(13.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(TEXT))
                            .child(match &store.auth {
                                crate::sync::AuthState::Starting => "Getting things ready",
                                crate::sync::AuthState::Locked => "Unlock your keyring",
                                _ => "Your calendar is ready when you are",
                            }),
                    )
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(MUTED))
                            .child("Open the full window to finish setup."),
                    ),
            );
        } else if !has_loaded_range && store.busy {
            body = body.child(
                div()
                    .rounded_md()
                    .bg(rgb(BG))
                    .px_3()
                    .py_3()
                    .text_size(px(11.0))
                    .text_color(rgb(MUTED))
                    .child("Loading your calendar…"),
            );
        } else {
            body = body
                .child(self.day_section(store, self.selected, today, &tz, cx))
                .child(self.day_section(store, add_days(self.selected, 1), today, &tz, cx));
        }

        div()
            .id("tray-agenda-scroll")
            .flex_1()
            .min_h(px(0.0))
            .overflow_y_scroll()
            .track_scroll(&self.agenda_scroll)
            .child(body)
    }
}

impl Render for TrayPopoverView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store_entity = self.store.clone();
        let store = store_entity.read(cx);
        let today = Zoned::now().date();
        let date_title = if self.selected == today {
            "A LITTLE LOOK AHEAD".to_string()
        } else {
            self.selected
                .strftime("%A, %-d %B")
                .to_string()
                .to_uppercase()
        };
        let date_badge = if self.selected == today {
            "TODAY"
        } else {
            "DAY VIEW"
        };
        let week = self.week;
        let week_strip = self.week_strip(store, cx);
        let agenda = self.agenda(store, cx);

        div()
            .key_context("AntarcticalTray")
            .on_action(cx.listener(|_, _: &Back, window, _| window.remove_window()))
            .size_full()
            .bg(rgb(BG))
            .p(px(1.0))
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .overflow_hidden()
                    .rounded_lg()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(PANEL))
                    .child(
                        div()
                            .h(px(50.0))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .border_b_1()
                            .border_color(rgb(BORDER))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .child(branding::icon(32.0))
                                    .child(
                                        div()
                                            .text_size(px(12.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(TEXT))
                                            .child("antarctical"),
                                    )
                                    .child(
                                        div()
                                            .rounded_full()
                                            .bg(rgb(SELECTED))
                                            .px_2()
                                            .py(px(2.0))
                                            .text_size(px(9.0))
                                            .font_weight(FontWeight::BOLD)
                                            .text_color(rgb(ACCENT_TEXT))
                                            .child(date_badge),
                                    ),
                            )
                            .child(ghost_button(
                                "close-tray-popover",
                                "×",
                                Some(Box::new(|_, window, _| window.remove_window())),
                            )),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .pt_3()
                            .pb_2()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.0))
                                    .child(
                                        div()
                                            .text_size(px(17.0))
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(rgb(TEXT))
                                            .child(week_title(week)),
                                    )
                                    .child(
                                        div()
                                            .text_size(px(10.0))
                                            .text_color(rgb(MUTED))
                                            .child("YOUR WEEK"),
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(ghost_button(
                                        "tray-prev-week",
                                        "‹",
                                        Some(Box::new(
                                            cx.listener(|this, _, _, cx| this.move_week(-1, cx)),
                                        )),
                                    ))
                                    .child(ghost_button(
                                        "tray-next-week",
                                        "›",
                                        Some(Box::new(
                                            cx.listener(|this, _, _, cx| this.move_week(1, cx)),
                                        )),
                                    )),
                            ),
                    )
                    .child(div().px_4().child(week_strip))
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .justify_between()
                            .px_4()
                            .pt_3()
                            .pb_1()
                            .border_t_1()
                            .border_color(rgb(BORDER))
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(rgb(ACCENT_TEXT))
                                    .child(date_title),
                            )
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .text_color(rgb(MUTED))
                                    .child(if store.busy { "SYNCING" } else { "AGENDA" }),
                            ),
                    )
                    .child(agenda)
                    .child(
                        div()
                            .h(px(56.0))
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .px_4()
                            .border_t_1()
                            .border_color(rgb(BORDER))
                            .child(
                                button(
                                    "open-full-calendar",
                                    "Open full calendar  ↗",
                                    ButtonStyle::Primary,
                                    Some(Box::new(cx.listener(|this, _, window, cx| {
                                        this.open_full(this.selected, window, cx)
                                    }))),
                                )
                                .w_full(),
                            ),
                    ),
            )
    }
}

/// Toggles the compact agenda from the system tray.
pub fn toggle(store: &Entity<Store>, handle: &TrayWindow, click: Option<(i32, i32)>, cx: &mut App) {
    let existing = handle.borrow_mut().take();
    if let Some(existing) = existing {
        let closed = match existing.update(cx, |_, window, _| window.remove_window()) {
            Ok(()) => true,
            Err(error) => {
                eprintln!("antarctical: could not close the existing tray calendar: {error}");
                false
            }
        };
        store.read(cx).send(Command::Forget(ViewId::Tray));
        if closed {
            return;
        }
    }
    open(store, handle, click, cx);
}

/// Closes the compact agenda before opening the full calendar.
pub fn close(store: &Entity<Store>, handle: &TrayWindow, cx: &mut App) {
    let existing = handle.borrow_mut().take();
    if let Some(existing) = existing {
        if let Err(error) = existing.update(cx, |_, window, _| window.remove_window()) {
            eprintln!("antarctical: could not close the tray calendar: {error}");
        }
        store.read(cx).send(Command::Forget(ViewId::Tray));
    }
}

fn open(store: &Entity<Store>, handle: &TrayWindow, click: Option<(i32, i32)>, cx: &mut App) {
    // Some StatusNotifier hosts (including Quickshell) send Activate(0, 0)
    // instead of the pointer position, so use Niri's focused output as a fallback.
    let click = click.filter(|position| *position != (0, 0));
    let niri_output = focused_niri_output(cx);
    let click_display = if niri_output.is_none() {
        click.and_then(|(x, y)| {
            cx.displays().into_iter().find(|display| {
                let bounds = display.bounds();
                px(x as f32) >= bounds.origin.x
                    && px(x as f32) < bounds.origin.x + bounds.size.width
                    && px(y as f32) >= bounds.origin.y
                    && px(y as f32) < bounds.origin.y + bounds.size.height
            })
        })
    } else {
        None
    };
    let display = niri_output
        .as_ref()
        .map(|(display, _)| display.clone())
        .or(click_display)
        .or_else(|| cx.primary_display());
    let niri_bounds = niri_output.map(|(_, bounds)| bounds);

    open_on_display(store, handle, click, display, niri_bounds, cx);
}

fn open_on_display(
    store: &Entity<Store>,
    handle: &TrayWindow,
    click: Option<(i32, i32)>,
    display: Option<Rc<dyn PlatformDisplay>>,
    niri_bounds: Option<Bounds<gpui::Pixels>>,
    cx: &mut App,
) {
    let size = size(px(POPOVER_WIDTH), px(POPOVER_HEIGHT));
    let bounds = niri_bounds
        .map(|output| popup_bounds(output, size, click, px(56.0)))
        .or_else(|| {
            display
                .as_ref()
                .map(|display| popup_bounds(display.visible_bounds(), size, click, px(12.0)))
        })
        .unwrap_or_else(|| Bounds::centered(None, size, cx));
    #[cfg(target_os = "linux")]
    let kind = if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        let (left_margin, top_margin) = niri_bounds
            .map(|output| {
                (
                    (bounds.origin.x - output.origin.x).max(px(0.0)),
                    (bounds.origin.y - output.origin.y).max(px(0.0)),
                )
            })
            .or_else(|| {
                display.as_ref().map(|display| {
                    let screen = display.bounds();
                    let left = (bounds.origin.x - screen.origin.x).max(px(0.0));
                    let top = (bounds.origin.y - screen.origin.y).max(px(56.0));
                    (left, top)
                })
            })
            .unwrap_or((px(12.0), px(56.0)));
        WindowKind::LayerShell(LayerShellOptions {
            namespace: "korora.tech.Antarctical.tray".to_string(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::LEFT,
            exclusive_zone: Some(px(0.0)),
            margin: Some((top_margin, px(0.0), px(0.0), left_margin)),
            keyboard_interactivity: KeyboardInteractivity::OnDemand,
            ..Default::default()
        })
    } else {
        WindowKind::PopUp
    };
    #[cfg(not(target_os = "linux"))]
    let kind = WindowKind::PopUp;

    let options = gpui::WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        focus: true,
        show: true,
        kind,
        is_movable: true,
        is_resizable: false,
        is_minimizable: false,
        display_id: display.as_ref().map(|display| display.id()),
        window_background: WindowBackgroundAppearance::Opaque,
        app_id: Some(branding::APP_ID.into()),
        window_min_size: Some(size),
        window_decorations: Some(WindowDecorations::Client),
        ..Default::default()
    };
    let store_for_view = store.clone();
    match cx.open_window(options, move |_, cx| {
        cx.new(|cx| TrayPopoverView::new(store_for_view, cx))
    }) {
        Ok(window) => {
            let id = window.window_id();
            *handle.borrow_mut() = Some(window);
            let handle = handle.clone();
            let store = store.clone();
            cx.on_window_closed(move |cx, closed| {
                let is_current = handle
                    .borrow()
                    .as_ref()
                    .is_some_and(|current| current.window_id() == id);
                if closed == id && is_current {
                    handle.borrow_mut().take();
                    store.read(cx).send(Command::Forget(ViewId::Tray));
                }
            })
            .detach();
            if let Err(error) = window.update(cx, |view, window, cx| window.focus(&view.focus, cx))
            {
                eprintln!("antarctical: could not focus the tray calendar: {error}");
            }
            cx.activate(true);
        }
        Err(e) => eprintln!("antarctical: could not open tray calendar: {e}"),
    }
}

fn popup_bounds(
    area: Bounds<gpui::Pixels>,
    size: gpui::Size<gpui::Pixels>,
    click: Option<(i32, i32)>,
    top_inset: gpui::Pixels,
) -> Bounds<gpui::Pixels> {
    let margin = px(12.0);
    let left = area.origin.x + margin;
    let top = area.origin.y + top_inset.max(margin);
    let right = (area.origin.x + area.size.width - size.width - margin).max(left);
    let bottom = (area.origin.y + area.size.height - size.height - margin).max(top);
    let origin = click
        .map(|(x, y)| {
            let anchor = point(px(x as f32), px(y as f32));
            let right_half = anchor.x > area.center().x;
            let bottom_half = anchor.y > area.center().y;
            let x = if right_half {
                anchor.x - size.width
            } else {
                anchor.x
            };
            let y = if bottom_half {
                anchor.y - size.height
            } else {
                anchor.y + px(18.0)
            };
            point(x.clamp(left, right), y.clamp(top, bottom))
        })
        .unwrap_or_else(|| point(right, top));
    Bounds::new(origin, size)
}

#[cfg(target_os = "linux")]
fn focused_niri_output(cx: &App) -> Option<(Rc<dyn PlatformDisplay>, Bounds<gpui::Pixels>)> {
    let current_desktop = std::env::var("XDG_CURRENT_DESKTOP").ok()?;
    if !current_desktop
        .split(':')
        .any(|desktop| desktop.eq_ignore_ascii_case("niri"))
    {
        return None;
    }

    let output = std::process::Command::new("niri")
        .args(["msg", "-j", "focused-output"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }

    let focused: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
    let output_name = focused.get("name")?.as_str()?;
    let logical = focused.get("logical")?;
    let number = |key: &str| logical.get(key)?.as_f64().map(|value| value as f32);
    let logical_bounds = Bounds::new(
        point(px(number("x")?), px(number("y")?)),
        size(px(number("width")?), px(number("height")?)),
    );
    let output_uuid = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, output_name.as_bytes());
    let display = cx
        .displays()
        .into_iter()
        .find(|display| display.uuid().is_ok_and(|uuid| uuid == output_uuid))?;
    Some((display, logical_bounds))
}

#[cfg(not(target_os = "linux"))]
fn focused_niri_output(_: &App) -> Option<(Rc<dyn PlatformDisplay>, Bounds<gpui::Pixels>)> {
    None
}
