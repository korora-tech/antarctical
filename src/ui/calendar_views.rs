//! The month grid, the week/day time grid and the agenda list.

use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, Div, FontWeight, SharedString, Stateful, Window, div, px,
    relative, rgb,
};
use jiff::civil::{Date, Weekday};
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

use super::format;
use super::main_view::{AGENDA_DAYS, MainView};
use super::theme::{
    ACCENT, ACCENT_TEXT, AGENDA_GUTTER_W, AGENDA_MAX_W, AGENDA_RADIUS, AGENDA_ROW_H,
    AGENDA_SECTION_GAP, AGENDA_TIME_W, ALL_DAY_BAR_W, ALL_DAY_FILL, ALL_DAY_FILL_DECLINED,
    ALL_DAY_FILL_HOVER, BG, BORDER, CAPTION, CHIP_TIME_W_12H, CHIP_TIME_W_24H, DAY_HEADER_H,
    DISABLED, GRID_LINE, HEADER_H, HOUR_PX, HOVER, MONTH_DAY_PX, MUTED, NOW_LINE, NOW_LINE_FAINT,
    NOW_PILL_CLEARANCE, NOW_PILL_H, NOW_PILL_TEXT, ON_ACCENT, OUTSIDE, PANEL, PAST_DAY_TEXT,
    PAST_HOVER_OPACITY, PAST_OPACITY, SECONDARY, SELECTED, TEXT, TODAY_BG, TODAY_HEADER_BG,
    TODAY_HOVER, WEEKDAY_HEADER_H, WEEKEND_BG, tabular, tint,
};
use crate::layout;
use crate::model;
use crate::model::Event;
use crate::store::Store;

const MONTH_ROW_H: f32 = 19.0;
const DAY_MINUTES: i32 = 24 * 60;
const GUTTER_W: f32 = 56.0;
/// Right inset shared by hour labels and the all-day label.
const GUTTER_PAD: f32 = 8.0;

/// A chip's id and whether its bar joins the same event in the neighbouring cells.
struct ChipSlot {
    id: SharedString,
    join_left: bool,
    join_right: bool,
    /// The parent cell's padding, which a joined side reaches across.
    pad: f32,
}

impl ChipSlot {
    /// A chip that stands alone in its cell.
    fn single(id: SharedString, pad: f32) -> Self {
        Self {
            id,
            join_left: false,
            join_right: false,
            pad,
        }
    }
}

/// Whether `e` sits at slot `k` of a neighbouring column's rendered events.
fn joins(neighbour: Option<&[&Event]>, k: usize, e: &Event) -> bool {
    // events_between yields each event once, so identity means the same event.
    neighbour
        .and_then(|n| n.get(k))
        .is_some_and(|p| std::ptr::eq(*p, e))
}

/// Refines ties after the store's order (all-day, then start): longer events first, so
/// multi-day ribbons keep their slot across cells where they can.
fn ribbon_order(a: &Event, b: &Event, tz: &TimeZone) -> std::cmp::Ordering {
    let (sa, sb) = (a.instants(tz), b.instants(tz));
    b.is_all_day()
        .cmp(&a.is_all_day())
        .then(sa.start.cmp(&sb.start))
        .then(sb.end.cmp(&sa.end))
        .then_with(|| a.title.cmp(&b.title))
        .then_with(|| a.calendar_id.cmp(&b.calendar_id))
        .then_with(|| a.id.cmp(&b.id))
}

/// Whether the event has already ended.
fn is_past(e: &Event, now: Timestamp, tz: &TimeZone) -> bool {
    e.instants(tz).end <= now
}

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

    /// A one-line chip: a soft ribbon for all-day events, a dot + time for timed ones.
    fn chip(
        &self,
        store: &Store,
        e: &Event,
        date: Date,
        slot: ChipSlot,
        tz: &TimeZone,
        cx: &Context<Self>,
    ) -> Stateful<Div> {
        let color = store.color_of(e);
        let selected = self.is_selected(e);
        let declined = e.declined();
        // Reads the clock per chip rather than taking `now`: an 8th argument trips clippy's
        // too_many_arguments.
        let faded = is_past(e, Timestamp::now(), tz) && !selected && !declined;
        let base = div()
            .id(slot.id)
            .h(px(MONTH_ROW_H - 2.0))
            .flex()
            .flex_shrink_0()
            .items_center()
            .gap(px(5.0))
            .px(px(5.0))
            .overflow_hidden()
            .text_size(px(11.0))
            .border_1()
            .border_color(gpui::transparent_black())
            .when(selected, |d| d.border_color(rgb(ACCENT)))
            .when(faded, |d| d.opacity(PAST_OPACITY))
            .on_click(self.event_click(e, cx));
        if e.is_all_day() {
            let (join_left, join_right, pad) = (slot.join_left, slot.join_right, slot.pad);
            let fill = if declined {
                ALL_DAY_FILL_DECLINED
            } else {
                ALL_DAY_FILL
            };
            base.bg(rgb(tint(color, PANEL, fill)))
                .hover(move |s| {
                    let s = s
                        .bg(rgb(tint(color, PANEL, ALL_DAY_FILL_HOVER)))
                        .cursor_pointer();
                    if faded {
                        s.opacity(PAST_HOVER_OPACITY)
                    } else {
                        s
                    }
                })
                .when(!join_left, |d| d.rounded_l(px(4.0)))
                .when(!join_right, |d| d.rounded_r(px(4.0)))
                // Joined sides reach the cell edge, so one event reads as one ribbon.
                .when(join_left, |d| d.ml(px(-pad)).border_l_0())
                .when(join_right, |d| d.mr(px(-pad)).border_r_0())
                // Only the head carries the accent bar and the title.
                .when(!join_left, |d| {
                    d.relative()
                        .pl(px(ALL_DAY_BAR_W + 5.0))
                        .child(
                            div()
                                .absolute()
                                .top(px(-1.0))
                                .bottom(px(-1.0))
                                .left(px(-1.0))
                                .w(px(ALL_DAY_BAR_W))
                                .rounded_l(px(4.0))
                                .bg(rgb(color))
                                .when(declined, |d| d.opacity(0.4)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w(px(0.0))
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(rgb(if declined { MUTED } else { TEXT }))
                                .when(declined, |d| d.line_through())
                                .child(e.title.clone()),
                        )
                })
        } else {
            let start = e.start_zoned(tz);
            // On the days after an overnight event started, its start time would mislead.
            let label = if start.date() == date {
                format::time(&start, store.prefs.clock_24h)
            } else {
                "…".to_string()
            };
            let time_w = if store.prefs.clock_24h {
                CHIP_TIME_W_24H
            } else {
                CHIP_TIME_W_12H
            };
            base.rounded_sm()
                .hover(move |s| {
                    let s = s.bg(rgb(HOVER)).cursor_pointer();
                    if faded {
                        s.opacity(PAST_HOVER_OPACITY)
                    } else {
                        s
                    }
                })
                .child(
                    div()
                        .size(px(6.0))
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(rgb(color)),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .min_w(px(time_w))
                        .text_size(px(10.5))
                        .text_color(rgb(CAPTION))
                        .child(label),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(rgb(if declined { MUTED } else { TEXT }))
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
        let grid = model::month_grid(self.anchor);
        let month = self.anchor.month();
        let mut events = store.events_between(grid[0], model::add_days(grid[41], 1), &tz);
        events.sort_by(|a, b| ribbon_order(a, b, &tz));
        // Rows that fit in a cell: the window height minus chrome, over six weeks.
        let viewport = f32::from(window.viewport_size().height);
        let cell_h = ((viewport - HEADER_H - WEEKDAY_HEADER_H) / 6.0).max(60.0);
        // 1px bottom border, 4px padding top and bottom plus the 22px numeral; each row adds
        // its 2px gap and chip (MONTH_ROW_H).
        let rows = (((cell_h - 31.0) / MONTH_ROW_H).floor() as usize).max(1);

        // Labels follow month_grid's week start; px(10) lines up with the numerals.
        let weekday_header = div()
            .h(px(WEEKDAY_HEADER_H))
            .flex_shrink_0()
            .flex()
            .border_b_1()
            .border_color(rgb(BORDER))
            .children(grid[..7].iter().map(|d| {
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .px(px(10.0))
                    .text_size(px(10.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(CAPTION))
                    .child(d.strftime("%a").to_string().to_uppercase())
            }));

        let mut weeks = div().flex_1().min_h(px(0.0)).flex().flex_col();
        for week in grid.chunks(7) {
            let mut row = div().flex_1().min_h(px(0.0)).flex();
            // First pass: what each cell renders, so ribbons join only where both sides show.
            let cells: Vec<(Vec<&Event>, usize)> = week
                .iter()
                .map(|&date| {
                    let day_events: Vec<&Event> = events
                        .iter()
                        .copied()
                        .filter(|e| e.occurs_on(date, &tz))
                        .collect();
                    let overflow = day_events.len().saturating_sub(rows);
                    let shown = if overflow > 0 {
                        rows.saturating_sub(1)
                    } else {
                        rows
                    };
                    let shown = shown.min(day_events.len());
                    (day_events, shown)
                })
                .collect();
            let rendered: Vec<&[&Event]> = cells.iter().map(|(evs, n)| &evs[..*n]).collect();
            for (i, &date) in week.iter().enumerate() {
                let (day_events, shown) = (&cells[i].0, cells[i].1);
                let hidden = day_events.len().saturating_sub(shown);
                let is_today = date == today;
                let in_month = date.month() == month;
                // Today, then outside-month, past, weekend and other days; first match wins.
                let (fg, weight) = if is_today {
                    (ON_ACCENT, FontWeight::SEMIBOLD)
                } else if !in_month {
                    (DISABLED, FontWeight::NORMAL)
                } else if date < today {
                    (PAST_DAY_TEXT, FontWeight::NORMAL)
                } else if matches!(date.weekday(), Weekday::Saturday | Weekday::Sunday) {
                    (SECONDARY, FontWeight::MEDIUM)
                } else {
                    (TEXT, FontWeight::MEDIUM)
                };
                let numeral = div()
                    .h(px(22.0))
                    .min_w(px(22.0))
                    .px(px(6.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .text_size(px(MONTH_DAY_PX))
                    .font_features(tabular())
                    .font_weight(weight)
                    .text_color(rgb(fg))
                    .when(is_today, |d| d.bg(rgb(ACCENT)))
                    .child(date.day().to_string());
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
                    .bg(rgb(if is_today {
                        TODAY_BG
                    } else if in_month {
                        BG
                    } else {
                        OUTSIDE
                    }))
                    .hover(move |s| s.bg(rgb(if is_today { TODAY_HOVER } else { PANEL })))
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.open_day(date, cx)),
                    )
                    .child(if date.day() == 1 {
                        // A landmark: the month name keeps its accent on past month starts.
                        div()
                            .flex()
                            .items_center()
                            .gap(px(4.0))
                            .child(numeral)
                            .child(
                                div()
                                    .text_size(px(MONTH_DAY_PX))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(if in_month { ACCENT_TEXT } else { DISABLED }))
                                    .child(date.strftime("%b").to_string()),
                            )
                    } else {
                        div().flex().child(numeral)
                    })
                    .children(rendered[i].iter().enumerate().map(|(k, e)| {
                        let id = SharedString::from(format!("m-{date}-{}-{}", e.calendar_id, e.id));
                        let slot = if e.is_all_day() {
                            ChipSlot {
                                id,
                                join_left: i > 0 && joins(rendered.get(i - 1).copied(), k, e),
                                join_right: joins(rendered.get(i + 1).copied(), k, e),
                                pad: 4.0,
                            }
                        } else {
                            ChipSlot::single(id, 4.0)
                        };
                        self.chip(store, e, date, slot, &tz, cx)
                    }))
                    .when(hidden > 0, |d| {
                        d.child(
                            div()
                                .h(px(MONTH_ROW_H - 2.0))
                                .flex()
                                .items_center()
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
        let now_ts = now.timestamp();
        let today = now.date();
        let clock = store.prefs.clock_24h;
        let dates: Vec<Date> = (0..days).map(|i| model::add_days(first, i)).collect();
        let mut events = store.events_between(first, model::add_days(first, days), &tz);
        events.sort_by(|a, b| ribbon_order(a, b, &tz));
        let now_min = now.hour() as i32 * 60 + now.minute() as i32;
        let now_top = now_min as f32 * HOUR_PX / 60.0;
        let shows_now = dates.contains(&today);

        if self.pending_grid_scroll {
            self.pending_grid_scroll = false;
            // Today: an hour before now; other days: 07:00.
            let minutes = if shows_now {
                (now_min - 60).clamp(0, 16 * 60)
            } else {
                7 * 60
            };
            self.grid_scroll
                .set_offset(gpui::point(px(0.), px(-(minutes as f32) * HOUR_PX / 60.0)));
        }

        let day_header = div()
            .h(px(DAY_HEADER_H))
            .flex_shrink_0()
            .flex()
            .border_b_1()
            .border_color(rgb(BORDER))
            .child(div().w(px(GUTTER_W)).flex_shrink_0())
            .children(dates.iter().map(|&d| {
                let is_today = d == today;
                let is_past = d < today;
                let is_weekend = matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday);
                let wash = days > 1;
                // Today > past > weekend > default; past days recede most.
                let (label_fg, number_fg) = if is_today {
                    (ACCENT_TEXT, ON_ACCENT)
                } else if is_past {
                    (CAPTION, CAPTION)
                } else if is_weekend {
                    (MUTED, SECONDARY)
                } else {
                    (SECONDARY, TEXT)
                };
                div()
                    .id(SharedString::from(format!("hdr-{d}")))
                    .flex_1()
                    .min_w(px(0.0))
                    .overflow_hidden()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .px(px(8.0))
                    .border_l_1()
                    .border_color(rgb(GRID_LINE))
                    .when(wash && is_today, |el| el.bg(rgb(TODAY_HEADER_BG)))
                    .when(wash && is_weekend && !is_today, |el| el.bg(rgb(WEEKEND_BG)))
                    .when(days > 1, |el| {
                        el.hover(move |s| {
                            s.bg(rgb(if is_today { TODAY_HOVER } else { PANEL }))
                                .cursor_pointer()
                        })
                        .on_click(
                            cx.listener(move |this, _: &ClickEvent, _, cx| this.open_day(d, cx)),
                        )
                    })
                    // In narrow columns the caps give way first, so the date always shows;
                    // 8px padding, 4px gap and the 28px numeral fit the 960px window with both panes.
                    .child(
                        div()
                            .min_w(px(0.0))
                            .truncate()
                            .text_size(px(11.0))
                            .line_height(px(28.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(label_fg))
                            .child(d.strftime("%a").to_string().to_uppercase()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .h(px(28.0))
                            .min_w(px(28.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .text_size(px(20.0))
                            .line_height(px(28.0))
                            .text_color(rgb(number_fg))
                            .when(is_today, |s| {
                                s.rounded_full()
                                    .px(px(2.0))
                                    .bg(rgb(ACCENT))
                                    .font_weight(FontWeight::SEMIBOLD)
                            })
                            .when(!is_today, |s| s.font_weight(FontWeight::NORMAL))
                            .child(d.day().to_string()),
                    )
                    .when(days == 1, |el| {
                        el.child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(FontWeight::NORMAL)
                                .text_color(rgb(CAPTION))
                                .truncate()
                                .child(d.strftime("%B %Y").to_string()),
                        )
                    })
            }));

        // All-day strip: as tall as the busiest day, capped at 4 rows.
        let all_day: Vec<Vec<&Event>> = dates
            .iter()
            .map(|&d| {
                events
                    .iter()
                    .copied()
                    .filter(|e| e.is_all_day() && e.occurs_on(d, &tz))
                    .collect()
            })
            .collect();
        let max_rows = all_day.iter().map(Vec::len).max().unwrap_or(0).min(4);
        // What each column renders, so ribbons join only where both sides show.
        let rendered: Vec<&[&Event]> = all_day
            .iter()
            .map(|evs| {
                let shown = if evs.len() > max_rows {
                    max_rows - 1
                } else {
                    evs.len()
                };
                &evs[..shown]
            })
            .collect();
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
                        .pr(px(GUTTER_PAD))
                        .flex()
                        .justify_end()
                        .text_size(px(10.0))
                        .text_color(rgb(CAPTION))
                        .child("all-day"),
                )
                .children(dates.iter().enumerate().map(|(i, d)| {
                    let evs = &all_day[i];
                    let shown = rendered[i].len();
                    let hidden = evs.len() - shown;
                    let wash = days > 1;
                    let weekend = matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday);
                    div()
                        .flex_1()
                        .min_w(px(0.0))
                        .flex()
                        .flex_col()
                        .gap(px(2.0))
                        .p(px(3.0))
                        .overflow_hidden()
                        .border_l_1()
                        .border_color(rgb(GRID_LINE))
                        .when(wash && *d == today, |c| c.bg(rgb(TODAY_BG)))
                        .when(wash && weekend && *d != today, |c| c.bg(rgb(WEEKEND_BG)))
                        .children(rendered[i].iter().enumerate().map(|(k, e)| {
                            let slot = ChipSlot {
                                id: SharedString::from(format!(
                                    "ad-{d}-{}-{}",
                                    e.calendar_id, e.id
                                )),
                                join_left: i > 0 && joins(rendered.get(i - 1).copied(), k, e),
                                join_right: joins(rendered.get(i + 1).copied(), k, e),
                                pad: 3.0,
                            };
                            self.chip(store, e, *d, slot, &tz, cx)
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
            // Labels near now give way to the pill.
            .children(
                (1..24)
                    .filter(|&h| {
                        !shows_now || (h as f32 * HOUR_PX - now_top).abs() >= NOW_PILL_CLEARANCE
                    })
                    .map(|h| {
                        let (num, suffix) = format::hour_label_parts(h as i8, clock);
                        // A 12px line box topped 6px above the hour centres the label on its line.
                        div()
                            .absolute()
                            .top(px(h as f32 * HOUR_PX - 6.0))
                            .right(px(GUTTER_PAD))
                            .flex()
                            .items_baseline()
                            .line_height(px(12.0))
                            .text_color(rgb(CAPTION))
                            .child(
                                div()
                                    .text_size(px(10.0))
                                    .font_features(tabular())
                                    .child(num),
                            )
                            .when_some(suffix, |el, s| {
                                el.child(
                                    div()
                                        .ml(px(2.0))
                                        .text_size(px(9.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(s),
                                )
                            })
                    }),
            )
            .when(shows_now, |g| {
                // Clamped so the pill stays whole just after and just before midnight.
                g.child(
                    div()
                        .absolute()
                        .top(px(
                            (now_top - NOW_PILL_H / 2.0).clamp(0.0, 24.0 * HOUR_PX - NOW_PILL_H)
                        ))
                        .right(px(3.0))
                        .h(px(NOW_PILL_H))
                        .px(px(4.0))
                        .flex_none()
                        .whitespace_nowrap()
                        .rounded(px(4.0))
                        .bg(rgb(NOW_LINE))
                        .flex()
                        .items_center()
                        .text_size(px(10.0))
                        .line_height(px(12.0))
                        .font_weight(FontWeight::BOLD)
                        .font_features(tabular())
                        .text_color(rgb(NOW_PILL_TEXT))
                        .child(format::time(&now, clock)),
                )
            });

        let mut columns = div().flex_1().min_w(px(0.0)).flex().h_full();
        for &d in &dates {
            let day_from = model::day_start(d, &tz).timestamp();
            let day_to = model::day_start(model::add_days(d, 1), &tz).timestamp();
            let timed: Vec<&Event> = events
                .iter()
                .copied()
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
                    let span = e.instants(&tz);
                    let start = if span.start <= day_from {
                        0
                    } else {
                        minute_of(span.start)
                    };
                    let end = if span.end >= day_to {
                        DAY_MINUTES
                    } else {
                        minute_of(span.end)
                    };
                    // Short events stay clickable (20 min tall), without running off the day.
                    let end = end.max(start + 20).min(DAY_MINUTES);
                    (start.min(end - 20), end)
                })
                .collect();
            let slots = layout::place(&spans);

            let wash = days > 1;
            let weekend = matches!(d.weekday(), Weekday::Saturday | Weekday::Sunday);
            let mut col = div()
                .id(SharedString::from(format!("col-{d}")))
                .flex_1()
                .min_w(px(0.0))
                .relative()
                .h_full()
                .border_l_1()
                .border_color(rgb(GRID_LINE))
                .when(wash && d == today, |c| c.bg(rgb(TODAY_BG)))
                .when(wash && weekend && d != today, |c| c.bg(rgb(WEEKEND_BG)))
                .children((1..24).map(|h| {
                    div()
                        .absolute()
                        .top(px(h as f32 * HOUR_PX))
                        .left_0()
                        .right_0()
                        .h(px(1.0))
                        .bg(rgb(GRID_LINE))
                }))
                // Under the chips, so they cover it.
                .when(shows_now && d != today, |c| {
                    c.child(
                        div()
                            .absolute()
                            .top(px(now_top))
                            .left_0()
                            .right_0()
                            .h(px(1.0))
                            .bg(rgb(NOW_LINE_FAINT)),
                    )
                });
            for ((e, (start, end)), slot) in timed.iter().zip(spans).zip(slots) {
                let color = store.color_of(e);
                let declined = e.declined();
                let selected = self.is_selected(e);
                let faded = is_past(e, now_ts, &tz) && !selected && !declined;
                // gpui fades each primitive rather than a layer, so a faded block sits on an
                // opaque copy of the column bg to keep the hour lines from showing through.
                let underlay = if wash && d == today {
                    TODAY_BG
                } else if wash && weekend {
                    WEEKEND_BG
                } else {
                    BG
                };
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
                            div().size_full().rounded_md().bg(rgb(underlay)).child(
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
                                    .when(faded, |s| s.opacity(PAST_OPACITY))
                                    .hover(move |s| {
                                        let s = s.bg(rgb(tint(color, BG, 0.45))).cursor_pointer();
                                        if faded {
                                            s.opacity(PAST_HOVER_OPACITY)
                                        } else {
                                            s
                                        }
                                    })
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
                            ),
                        )
                        .on_click(self.event_click(e, cx)),
                );
            }
            if d == today {
                // After the chips, so the line draws on top. The head sits inside the column
                // so nothing clips it; the BG ring sets it apart from a chip's accent bar.
                col = col
                    .child(
                        div()
                            .absolute()
                            .top(px(now_top - 1.0))
                            .left_0()
                            .right_0()
                            .h(px(2.0))
                            .bg(rgb(NOW_LINE)),
                    )
                    .child(
                        div()
                            .absolute()
                            .left_0()
                            .top(px(now_top - 6.0))
                            .size(px(12.0))
                            .rounded_full()
                            .bg(rgb(NOW_LINE))
                            .border_2()
                            .border_color(rgb(BG)),
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
        let now = Zoned::now();
        let today = now.date();
        let now_ts = now.timestamp();
        let clock = store.prefs.clock_24h;
        let to = model::add_days(self.anchor, AGENDA_DAYS);
        let events = store.events_between(self.anchor, to, &tz);
        let mut list = div()
            .w_full()
            .max_w(px(AGENDA_MAX_W))
            .px_6()
            .py(px(AGENDA_SECTION_GAP))
            .flex()
            .flex_col()
            .gap(px(AGENDA_SECTION_GAP));
        let mut any = false;
        let mut prev_month: Option<(i16, i8)> = None;
        let mut d = self.anchor;
        while d < to {
            let day: Vec<&Event> = events
                .iter()
                .copied()
                .filter(|e| e.occurs_on(d, &tz))
                .collect();
            if !day.is_empty() {
                any = true;
                let show_month = prev_month != Some((d.year(), d.month()));
                prev_month = Some((d.year(), d.month()));
                let is_today = d == today;
                let caption = |text: String, color: u32| {
                    div()
                        .whitespace_nowrap()
                        .text_size(px(11.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(color))
                        .child(text)
                };
                let date = d;
                let gutter = div()
                    .id(SharedString::from(format!("ag-day-{d}")))
                    .w(px(AGENDA_GUTTER_W))
                    .flex_shrink_0()
                    .flex()
                    .flex_col()
                    .items_end()
                    .px(px(6.0))
                    .py(px(6.0))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(HOVER)))
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.open_day(date, cx)),
                    )
                    .when(show_month, |g| {
                        g.child(caption(
                            d.strftime("%b").to_string().to_uppercase(),
                            CAPTION,
                        ))
                    })
                    .when(is_today, |g| g.child(caption("Today".into(), ACCENT_TEXT)))
                    .when(d == model::add_days(today, 1), |g| {
                        g.child(caption("Tomorrow".into(), MUTED))
                    })
                    .child(
                        div()
                            .whitespace_nowrap()
                            .text_size(px(26.0))
                            .line_height(px(30.0))
                            .font_weight(FontWeight::BOLD)
                            .font_features(tabular())
                            .text_color(rgb(if is_today { ACCENT_TEXT } else { TEXT }))
                            .child(d.strftime("%-d").to_string()),
                    )
                    .child(caption(d.strftime("%a").to_string(), MUTED));
                let n = day.len();
                let group = div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .rounded(px(AGENDA_RADIUS))
                    .bg(rgb(PANEL))
                    .border_1()
                    .border_color(rgb(GRID_LINE))
                    .children(day.iter().enumerate().map(|(i, e)| {
                        let color = store.color_of(e);
                        let declined = e.declined();
                        let selected = self.is_selected(e);
                        let faded = is_past(e, now_ts, &tz) && !selected && !declined;
                        let cal = store
                            .calendar(&e.calendar_id)
                            .map(|c| c.name.clone())
                            .unwrap_or_default();
                        let sub = match &e.location {
                            Some(l) => format!("{cal} · {l}"),
                            None => cal,
                        };
                        let time_col = div()
                            .w(px(AGENDA_TIME_W))
                            .flex_shrink_0()
                            .flex()
                            .flex_col()
                            .items_end()
                            .whitespace_nowrap();
                        let time_col = match format::event_time_parts_on(e, d, &tz, clock) {
                            Some((main, second)) => time_col
                                .child(
                                    div()
                                        .text_size(px(13.0))
                                        .font_weight(FontWeight::MEDIUM)
                                        .font_features(tabular())
                                        .text_color(rgb(if declined { MUTED } else { SECONDARY }))
                                        .child(main),
                                )
                                .when_some(second, |c, second| {
                                    c.child(
                                        div()
                                            .text_size(px(11.0))
                                            .font_features(tabular())
                                            .text_color(rgb(MUTED))
                                            .child(second),
                                    )
                                }),
                            None => time_col.child(
                                div()
                                    .text_size(px(12.0))
                                    .font_weight(FontWeight::MEDIUM)
                                    .text_color(rgb(if declined { MUTED } else { SECONDARY }))
                                    .child("All day"),
                            ),
                        };
                        div()
                            .id(SharedString::from(format!(
                                "ag-{d}-{}-{}",
                                e.calendar_id, e.id
                            )))
                            .flex()
                            .items_center()
                            .gap_3()
                            .min_h(px(AGENDA_ROW_H))
                            .px(px(14.0))
                            .py(px(8.0))
                            .cursor_pointer()
                            .when(i > 0, |s| s.border_t_1().border_color(rgb(GRID_LINE)))
                            .when(i == 0, |s| s.rounded_t(px(AGENDA_RADIUS - 1.0)))
                            .when(i + 1 == n, |s| s.rounded_b(px(AGENDA_RADIUS - 1.0)))
                            .when(selected, |s| s.bg(rgb(SELECTED)))
                            .when(faded, |d| d.opacity(PAST_OPACITY))
                            .when(!selected, |s| {
                                s.hover(move |s| {
                                    let s = s.bg(rgb(HOVER));
                                    if faded {
                                        s.opacity(PAST_HOVER_OPACITY)
                                    } else {
                                        s
                                    }
                                })
                            })
                            .on_click(self.event_click(e, cx))
                            .child(time_col)
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
                                            .text_color(rgb(if declined { MUTED } else { TEXT }))
                                            .when(declined, |s| s.line_through())
                                            .child(e.title.clone()),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .mt(px(2.0))
                                            .text_size(px(11.0))
                                            .text_color(rgb(MUTED))
                                            .child(sub),
                                    ),
                            )
                    }));
                list = list.child(
                    div()
                        .flex()
                        .items_start()
                        .gap_4()
                        .child(gutter)
                        .child(group),
                );
            }
            d = model::add_days(d, 1);
        }
        if !any {
            list = list.child(
                div()
                    .pt_20()
                    .flex()
                    .justify_center()
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
            .child(div().w_full().flex().justify_center().child(list))
            .into_any_element()
    }
}
