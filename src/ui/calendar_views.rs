//! The month grid, the week/day time grid and the agenda list.

use gpui::{
    AnyElement, ClickEvent, Context, Div, FontWeight, SharedString, Stateful, Window, div,
    prelude::*, px, relative, rgb,
};
use jiff::Zoned;
use jiff::civil::Date;
use jiff::tz::TimeZone;

use super::format;
use super::main_view::{AGENDA_DAYS, MainView};
use super::theme::*;
use crate::layout;
use crate::model::{Event, add_days, day_start, month_grid};
use crate::store::Store;

const MONTH_ROW_H: f32 = 19.0;
const DAY_MINUTES: i32 = 24 * 60;
const GUTTER_W: f32 = 56.0;

impl MainView {
    fn event_click(
        &self,
        e: &Event,
        cx: &Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static {
        let e = e.clone();
        cx.listener(move |this, _: &ClickEvent, _, cx| {
            cx.stop_propagation();
            this.select(&e, cx);
        })
    }

    /// A one-line chip: a filled bar for all-day events, a dot + time for timed ones.
    fn chip(
        &self,
        store: &Store,
        e: &Event,
        date: Date,
        id: SharedString,
        tz: &TimeZone,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let color = store.color_of(e);
        let selected = self.is_selected(e);
        let declined = e.declined();
        let base = div()
            .id(id)
            .w_full()
            .h(px(MONTH_ROW_H - 2.0))
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(5.0))
            .px(px(5.0))
            .rounded_sm()
            .overflow_hidden()
            .text_size(px(11.0))
            .border_1()
            .border_color(gpui::transparent_black())
            .when(selected, |d| d.border_color(rgb(ACCENT)))
            .hover(|s| s.bg(rgb(HOVER)).cursor_pointer())
            .on_click(self.event_click(e, cx));
        if e.is_all_day() {
            base.bg(rgb(tint(color, PANEL, if declined { 0.15 } else { 0.45 })))
                .text_color(rgb(if declined { MUTED } else { TEXT }))
                .child(
                    div()
                        .truncate()
                        .when(declined, |d| d.line_through())
                        .child(e.title.clone()),
                )
        } else {
            let start = e.start_zoned(tz);
            // On the days after an overnight event started, its start time would mislead.
            let label = if start.date() == date {
                format::time(&start, store.prefs.clock_24h)
            } else {
                "…".to_string()
            };
            base.child(
                div()
                    .size(px(6.0))
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(rgb(color)),
            )
            .child(div().flex_shrink_0().text_color(rgb(MUTED)).child(label))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_color(rgb(if declined { MUTED } else { SECONDARY }))
                    .when(declined, |d| d.line_through())
                    .child(e.title.clone()),
            )
        }
    }

    pub(super) fn month_view(
        &mut self,
        store: &Store,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> AnyElement {
        let tz = TimeZone::system();
        let today = Zoned::now().date();
        let grid = month_grid(self.anchor);
        let month = self.anchor.month();
        let events = store.events_between(grid[0], add_days(grid[41], 1), &tz);
        // Rows that fit in a cell: the window height minus chrome, over six weeks.
        let viewport = f32::from(window.viewport_size().height);
        let cell_h = ((viewport - 52.0 - 28.0) / 6.0).max(60.0);
        let rows = (((cell_h - 26.0) / MONTH_ROW_H).floor() as usize).max(1);

        let weekday_header = div()
            .h(px(28.0))
            .flex_shrink_0()
            .flex()
            .border_b_1()
            .border_color(rgb(BORDER))
            .children(["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].map(|d| {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .px_2()
                    .text_size(px(11.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(rgb(CAPTION))
                    .child(d)
            }));

        let mut weeks = div().flex_1().min_h(px(0.0)).flex().flex_col();
        for week in grid.chunks(7) {
            let mut row = div().flex_1().min_h(px(0.0)).flex();
            for &date in week {
                let day_events: Vec<&Event> =
                    events.iter().filter(|e| e.occurs_on(date, &tz)).collect();
                let overflow = day_events.len().saturating_sub(rows);
                let shown = if overflow > 0 {
                    rows.saturating_sub(1)
                } else {
                    rows
                };
                let hidden = day_events.len().saturating_sub(shown);
                let is_today = date == today;
                let in_month = date.month() == month;
                let cell = div()
                    .id(SharedString::from(format!("day-{date}")))
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .p(px(4.0))
                    .overflow_hidden()
                    .border_r_1()
                    .border_b_1()
                    .border_color(rgb(GRID_LINE))
                    .bg(rgb(if in_month { BG } else { OUTSIDE }))
                    .hover(|s| s.bg(rgb(PANEL)))
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.open_day(date, cx)),
                    )
                    .child(
                        div().flex().child(
                            div()
                                .h(px(20.0))
                                .min_w(px(20.0))
                                .px(px(5.0))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .text_size(px(11.0))
                                .when(is_today, |d| {
                                    d.bg(rgb(ACCENT))
                                        .text_color(rgb(0x11111a))
                                        .font_weight(FontWeight::BOLD)
                                })
                                .when(!is_today, |d| {
                                    d.text_color(rgb(if in_month { SECONDARY } else { DISABLED }))
                                })
                                .child(if date.day() == 1 {
                                    date.strftime("%-d %b").to_string()
                                } else {
                                    date.day().to_string()
                                }),
                        ),
                    )
                    .children(day_events.iter().take(shown).map(|e| {
                        self.chip(
                            store,
                            e,
                            date,
                            SharedString::from(format!("m-{date}-{}-{}", e.calendar_id, e.id)),
                            &tz,
                            cx,
                        )
                    }))
                    .when(hidden > 0, |d| {
                        d.child(
                            div()
                                .px(px(5.0))
                                .text_size(px(11.0))
                                .text_color(rgb(MUTED))
                                .child(format!("+{hidden} more")),
                        )
                    });
                row = row.child(cell);
            }
            weeks = weeks.child(row);
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(weekday_header)
            .child(weeks)
            .into_any_element()
    }

    /// Week (`days = 7`) or day (`days = 1`) view starting at `first`.
    pub(super) fn time_grid(
        &mut self,
        store: &Store,
        first: Date,
        days: i32,
        cx: &Context<Self>,
    ) -> AnyElement {
        let tz = TimeZone::system();
        let now = Zoned::now();
        let today = now.date();
        let clock = store.prefs.clock_24h;
        let dates: Vec<Date> = (0..days).map(|i| add_days(first, i)).collect();
        let events = store.events_between(first, add_days(first, days), &tz);

        if self.pending_grid_scroll {
            self.pending_grid_scroll = false;
            // Today: an hour before now; other days: 07:00.
            let minutes = if dates.contains(&today) {
                (now.hour() as i32 * 60 + now.minute() as i32 - 60).clamp(0, 16 * 60)
            } else {
                7 * 60
            };
            self.grid_scroll
                .set_offset(gpui::point(px(0.), px(-(minutes as f32) * HOUR_PX / 60.0)));
        }

        let day_header = div()
            .h(px(44.0))
            .flex_shrink_0()
            .flex()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(div().w(px(GUTTER_W)).flex_shrink_0())
            .children(dates.iter().map(|&d| {
                let is_today = d == today;
                div()
                    .id(SharedString::from(format!("hdr-{d}")))
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .border_l_1()
                    .border_color(rgb(GRID_LINE))
                    .when(days > 1, |el| {
                        el.hover(|s| s.bg(rgb(PANEL)).cursor_pointer()).on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| this.open_day(d, cx)),
                        )
                    })
                    .child(
                        div()
                            .text_size(px(11.0))
                            .text_color(rgb(if is_today { ACCENT_TEXT } else { CAPTION }))
                            .child(d.strftime("%a").to_string().to_uppercase()),
                    )
                    .child(
                        div()
                            .h(px(26.0))
                            .min_w(px(26.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded_full()
                            .text_size(px(15.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .when(is_today, |s| s.bg(rgb(ACCENT)).text_color(rgb(0x11111a)))
                            .when(!is_today, |s| s.text_color(rgb(TEXT)))
                            .child(d.day().to_string()),
                    )
            }));

        // All-day strip: as tall as the busiest day, capped at 4 rows.
        let all_day: Vec<Vec<&Event>> = dates
            .iter()
            .map(|&d| {
                events
                    .iter()
                    .filter(|e| e.is_all_day() && e.occurs_on(d, &tz))
                    .collect()
            })
            .collect();
        let max_rows = all_day.iter().map(Vec::len).max().unwrap_or(0).min(4);
        let all_day_strip = (max_rows > 0).then(|| {
            div()
                .flex_shrink_0()
                .flex()
                .border_b_1()
                .border_color(rgb(BORDER))
                .child(
                    div()
                        .w(px(GUTTER_W))
                        .flex_shrink_0()
                        .pt(px(4.0))
                        .pr_2()
                        .flex()
                        .justify_end()
                        .text_size(px(10.0))
                        .text_color(rgb(CAPTION))
                        .child("all-day"),
                )
                .children(dates.iter().zip(&all_day).map(|(d, evs)| {
                    let shown = if evs.len() > max_rows {
                        max_rows - 1
                    } else {
                        evs.len()
                    };
                    let hidden = evs.len() - shown;
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .p(px(3.0))
                        .border_l_1()
                        .border_color(rgb(GRID_LINE))
                        .children(evs.iter().take(shown).map(|e| {
                            self.chip(
                                store,
                                e,
                                *d,
                                SharedString::from(format!("ad-{d}-{}-{}", e.calendar_id, e.id)),
                                &tz,
                                cx,
                            )
                        }))
                        .when(hidden > 0, |el| {
                            el.child(
                                div()
                                    .px(px(5.0))
                                    .text_size(px(11.0))
                                    .text_color(rgb(MUTED))
                                    .child(format!("+{hidden} more")),
                            )
                        })
                }))
        });

        let gutter = div()
            .w(px(GUTTER_W))
            .flex_shrink_0()
            .relative()
            .children((1..24).map(|h| {
                div()
                    .absolute()
                    .top(px(h as f32 * HOUR_PX - 7.0))
                    .right(px(8.0))
                    .text_size(px(10.0))
                    .text_color(rgb(CAPTION))
                    .child(format::hour_label(h as i8, clock))
            }));

        let mut columns = div().flex_1().min_w(px(0.0)).flex().h_full();
        for &d in &dates {
            let day_from = day_start(d, &tz).timestamp();
            let day_to = day_start(add_days(d, 1), &tz).timestamp();
            let timed: Vec<&Event> = events
                .iter()
                .filter(|e| !e.is_all_day() && e.occurs_on(d, &tz))
                .collect();
            // Minutes on the wall clock, so a 23- or 25-hour DST day still maps to 24 rows.
            let minute_of = |t: jiff::Timestamp| {
                let z = t.to_zoned(tz.clone());
                z.hour() as i32 * 60 + z.minute() as i32
            };
            let spans: Vec<(i32, i32)> = timed
                .iter()
                .map(|e| {
                    let (s, en) = e.instants(&tz);
                    let start = if s <= day_from { 0 } else { minute_of(s) };
                    let end = if en >= day_to {
                        DAY_MINUTES
                    } else {
                        minute_of(en)
                    };
                    // Short events stay clickable (20 min tall), without running off the day.
                    let end = end.max(start + 20).min(DAY_MINUTES);
                    (start.min(end - 20), end)
                })
                .collect();
            let slots = layout::place(&spans);

            let mut col = div()
                .id(SharedString::from(format!("col-{d}")))
                .flex_1()
                .min_w(px(0.0))
                .relative()
                .h_full()
                .border_l_1()
                .border_color(rgb(GRID_LINE))
                .when(d == today, |c| c.bg(rgb(0x10121a)))
                .children((1..24).map(|h| {
                    div()
                        .absolute()
                        .top(px(h as f32 * HOUR_PX))
                        .left_0()
                        .right_0()
                        .h(px(1.0))
                        .bg(rgb(GRID_LINE))
                }));
            for ((e, (start, end)), slot) in timed.iter().zip(spans).zip(slots) {
                let color = store.color_of(e);
                let declined = e.declined();
                let selected = self.is_selected(e);
                let top = start as f32 * HOUR_PX / 60.0;
                let height = ((end - start) as f32 * HOUR_PX / 60.0 - 2.0).max(14.0);
                let width = 1.0 / slot.cols as f32;
                col = col.child(
                    div()
                        .id(SharedString::from(format!(
                            "t-{d}-{}-{}",
                            e.calendar_id, e.id
                        )))
                        .absolute()
                        .top(px(top + 1.0))
                        .h(px(height))
                        .left(relative(slot.col as f32 * width))
                        .w(relative(width))
                        .pr(px(3.0))
                        .pl(px(2.0))
                        .child(
                            div()
                                .size_full()
                                .flex()
                                .flex_col()
                                .overflow_hidden()
                                .rounded_md()
                                .border_l_2()
                                .border_color(rgb(color))
                                .bg(rgb(tint(color, BG, if declined { 0.12 } else { 0.32 })))
                                .when(selected, |s| s.border_1().border_color(rgb(ACCENT)))
                                .px(px(6.0))
                                .py(px(if height < 24.0 { 0.0 } else { 2.0 }))
                                .hover(|s| s.bg(rgb(tint(color, BG, 0.45))).cursor_pointer())
                                .child(
                                    div()
                                        .line_height(px(14.0))
                                        .when(height < 24.0, |s| s.truncate())
                                        .text_size(px(11.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(rgb(if declined { MUTED } else { TEXT }))
                                        .when(declined, |s| s.line_through())
                                        .child(e.title.clone()),
                                )
                                .when(height > 30.0, |s| {
                                    s.child(
                                        div()
                                            .text_size(px(10.0))
                                            .text_color(rgb(SECONDARY))
                                            .child(format::event_time_on(e, d, &tz, clock)),
                                    )
                                })
                                .when(height > 46.0 && e.location.is_some(), |s| {
                                    s.child(
                                        div()
                                            .text_size(px(10.0))
                                            .text_color(rgb(MUTED))
                                            .truncate()
                                            .child(e.location.clone().unwrap_or_default()),
                                    )
                                }),
                        )
                        .on_click(self.event_click(e, cx)),
                );
            }
            if d == today {
                let top = (now.hour() as f32 * 60.0 + now.minute() as f32) * HOUR_PX / 60.0;
                col = col.child(
                    div()
                        .absolute()
                        .top(px(top - 1.0))
                        .left_0()
                        .right_0()
                        .h(px(2.0))
                        .bg(rgb(NOW_LINE))
                        .child(
                            div()
                                .absolute()
                                .left(px(-4.0))
                                .top(px(-3.0))
                                .size(px(8.0))
                                .rounded_full()
                                .bg(rgb(NOW_LINE)),
                        ),
                );
            }
            columns = columns.child(col);
        }

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(day_header)
            .children(all_day_strip)
            .child(
                div()
                    .id("time-grid")
                    .flex_1()
                    .min_h(px(0.0))
                    .overflow_y_scroll()
                    .track_scroll(&self.grid_scroll)
                    .child(
                        div()
                            .h(px(24.0 * HOUR_PX))
                            .w_full()
                            .flex()
                            .child(gutter)
                            .child(columns),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn agenda_view(&mut self, store: &Store, cx: &Context<Self>) -> AnyElement {
        let tz = TimeZone::system();
        let today = Zoned::now().date();
        let clock = store.prefs.clock_24h;
        let to = add_days(self.anchor, AGENDA_DAYS);
        let events = store.events_between(self.anchor, to, &tz);
        let mut list = div().flex().flex_col().gap_4().p_6().max_w(px(860.0));
        let mut any = false;
        let mut d = self.anchor;
        while d < to {
            let day: Vec<&Event> = events.iter().filter(|e| e.occurs_on(d, &tz)).collect();
            if !day.is_empty() {
                any = true;
                let label = if d == today {
                    format!("Today · {}", d.strftime("%A %-d %B"))
                } else if d == add_days(today, 1) {
                    format!("Tomorrow · {}", d.strftime("%A %-d %B"))
                } else {
                    d.strftime("%A %-d %B").to_string()
                };
                let date = d;
                list = list.child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .id(SharedString::from(format!("ag-day-{d}")))
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(if d == today { ACCENT_TEXT } else { SECONDARY }))
                                .pb_1()
                                .hover(|s| s.text_color(rgb(TEXT)).cursor_pointer())
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.open_day(date, cx)
                                }))
                                .child(label),
                        )
                        .children(day.iter().map(|e| {
                            let color = store.color_of(e);
                            let declined = e.declined();
                            let cal = store
                                .calendar(&e.calendar_id)
                                .map(|c| c.name.clone())
                                .unwrap_or_default();
                            let sub = match &e.location {
                                Some(l) => format!("{cal} · {l}"),
                                None => cal,
                            };
                            div()
                                .id(SharedString::from(format!(
                                    "ag-{d}-{}-{}",
                                    e.calendar_id, e.id
                                )))
                                .flex()
                                .items_center()
                                .gap_3()
                                .rounded_md()
                                .px_3()
                                .py_2()
                                .bg(rgb(if self.is_selected(e) { SELECTED } else { PANEL }))
                                .border_1()
                                .border_color(rgb(if self.is_selected(e) {
                                    SELECTED_BORDER
                                } else {
                                    BORDER
                                }))
                                .hover(|s| s.bg(rgb(HOVER)).cursor_pointer())
                                .on_click(self.event_click(e, cx))
                                .child(
                                    div()
                                        .w(px(120.0))
                                        .flex_shrink_0()
                                        .text_size(px(12.0))
                                        .text_color(rgb(SECONDARY))
                                        .child(format::event_time_on(e, d, &tz, clock)),
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
                                        .flex_1()
                                        .min_w(px(0.0))
                                        .flex()
                                        .flex_col()
                                        .child(
                                            div()
                                                .truncate()
                                                .text_size(px(13.0))
                                                .font_weight(FontWeight::MEDIUM)
                                                .text_color(rgb(if declined {
                                                    MUTED
                                                } else {
                                                    TEXT
                                                }))
                                                .when(declined, |s| s.line_through())
                                                .child(e.title.clone()),
                                        )
                                        .child(
                                            div()
                                                .truncate()
                                                .text_size(px(11.0))
                                                .text_color(rgb(MUTED))
                                                .child(sub),
                                        ),
                                )
                        })),
                );
            }
            d = add_days(d, 1);
        }
        if !any {
            list = list.child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgb(MUTED))
                    .child(format!(
                        "Nothing in the {AGENDA_DAYS} days from {}.",
                        self.anchor.strftime("%-d %B")
                    )),
            );
        }
        div()
            .id("agenda")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.agenda_scroll)
            .child(list)
            .into_any_element()
    }
}
