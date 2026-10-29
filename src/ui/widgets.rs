//! Stateless building blocks. Disabled controls never get an `on_click`.

use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    App, Bounds, ClickEvent, Div, ElementId, Font, FontWeight, GlobalElementId, Hsla, LayoutId,
    Pixels, ShapedLine, SharedString, Stateful, StrikethroughStyle, Style, TextAlign, TextRun,
    Window, div, point, px, relative, rgb, transparent_black,
};

use super::theme::{
    ACCENT, ACCENT_TEXT, BORDER, CAL_DOT, CAL_ROW_H, CAPTION, CONTROL_H, DISABLED, EVENT_LINE_H,
    EVENT_META_LINE_H, EVENT_META_PX, EVENT_TITLE_PX, HOVER, MUTED, ON_ACCENT, PANEL, PANEL_RAISED,
    RED, SECONDARY, SEGMENT_ACTIVE, SEGMENT_ACTIVE_BORDER, SEGMENT_HOVER, SEGMENT_TRACK, TEXT,
    tabular,
};

pub type OnClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    Primary,
    Danger,
    Secondary,
    /// Outlined header control at CONTROL_H.
    Toolbar,
}

/// A pill button. `on_click: None` renders it disabled.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    style: ButtonStyle,
    on_click: Option<OnClick>,
) -> Stateful<Div> {
    let enabled = on_click.is_some();
    let toolbar = matches!(style, ButtonStyle::Toolbar);
    let (bg, fg, border): (Hsla, u32, u32) = match (style, enabled) {
        (_, false) => (rgb(PANEL).into(), DISABLED, BORDER),
        (ButtonStyle::Primary, true) => (rgb(ACCENT).into(), ON_ACCENT, ACCENT),
        (ButtonStyle::Danger, true) => (rgb(PANEL_RAISED).into(), RED, 0x5a2a32),
        (ButtonStyle::Secondary, true) => (rgb(PANEL_RAISED).into(), TEXT, 0x303646),
        (ButtonStyle::Toolbar, true) => (transparent_black(), SECONDARY, BORDER),
    };
    let el = div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded_md()
        .border_1()
        .border_color(rgb(border))
        .bg(bg)
        .px_3()
        .py(px(5.0))
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .when(toolbar, |d| {
            d.h(px(CONTROL_H))
                .py_0()
                .rounded(px(8.0))
                .font_weight(FontWeight::MEDIUM)
        })
        .text_color(rgb(fg))
        .child(label.into());
    match on_click {
        Some(handler) if toolbar => el
            .hover(|s| s.bg(rgb(HOVER)).text_color(rgb(TEXT)).cursor_pointer())
            .on_click(handler),
        Some(handler) => el
            .hover(|s| s.opacity(0.88).cursor_pointer())
            .on_click(handler),
        None => el,
    }
}

/// A borderless text button (chevrons, close, links).
pub fn ghost_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on_click: Option<OnClick>,
) -> Stateful<Div> {
    let el = div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded_md()
        .px(px(8.0))
        .py(px(4.0))
        .text_size(px(13.0))
        .text_color(rgb(if on_click.is_some() {
            SECONDARY
        } else {
            DISABLED
        }))
        .child(label.into());
    match on_click {
        Some(handler) => el
            .hover(|s| s.bg(rgb(HOVER)).text_color(rgb(TEXT)).cursor_pointer())
            .on_click(handler),
        None => el,
    }
}

/// A recessed well holding segments (view switcher, date navigation).
pub fn control_group() -> Div {
    div()
        .h(px(CONTROL_H))
        .flex_shrink_0()
        .flex()
        .items_center()
        .p(px(2.0))
        .gap(px(2.0))
        .rounded(px(8.0))
        .bg(rgb(SEGMENT_TRACK))
}

/// One option of a segmented control, inset 2px in its `control_group`. Every segment keeps a
/// 1px border (transparent unless active) so switching never shifts the layout. Callers set the
/// width. Without `on_click` the segment is inert: no hover, no pointer.
pub fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    on_click: Option<OnClick>,
) -> Stateful<Div> {
    let el = div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .h(px(CONTROL_H - 4.0))
        .rounded(px(6.0))
        .border_1()
        .text_size(px(12.0));
    let el = if active {
        el.bg(rgb(SEGMENT_ACTIVE))
            .border_color(rgb(SEGMENT_ACTIVE_BORDER))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(rgb(TEXT))
    } else {
        el.border_color(transparent_black())
            .font_weight(FontWeight::MEDIUM)
            .text_color(rgb(MUTED))
    };
    let el = match on_click {
        Some(handler) if active => el.hover(|s| s.cursor_pointer()).on_click(handler),
        Some(handler) => el
            .hover(|s| {
                s.bg(rgb(SEGMENT_HOVER))
                    .text_color(rgb(SECONDARY))
                    .cursor_pointer()
            })
            .on_click(handler),
        None => el,
    };
    el.child(label.into())
}

/// A compact icon-width segment (‹ ›) matching an inactive segment.
pub fn chevron_segment(
    id: impl Into<ElementId>,
    glyph: impl Into<SharedString>,
    on_click: OnClick,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h_full()
        .w(px(26.0))
        .rounded(px(6.0))
        .text_size(px(14.0))
        .text_color(rgb(MUTED))
        .hover(|s| {
            s.bg(rgb(SEGMENT_HOVER))
                .text_color(rgb(SECONDARY))
                .cursor_pointer()
        })
        .on_click(on_click)
        .child(glyph.into())
}

/// An inline text link.
pub fn link(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    on_click: OnClick,
) -> Stateful<Div> {
    div()
        .id(id)
        .text_size(px(12.0))
        .text_color(rgb(ACCENT_TEXT))
        .hover(|s| s.underline().cursor_pointer())
        .on_click(on_click)
        .child(label.into())
}

pub fn caption(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(10.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(CAPTION))
        .child(text.into())
}

/// A checkbox tinted with `color` (preferences in Settings).
pub fn checkbox_row(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    color: u32,
    checked: bool,
    on_click: Option<OnClick>,
) -> Stateful<Div> {
    let row = div()
        .id(id)
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .rounded_md()
        .px_2()
        .py(px(5.0));
    let row = match on_click {
        Some(handler) => row
            .hover(|s| s.bg(rgb(HOVER)).cursor_pointer())
            .on_click(handler),
        None => row,
    };
    row.child(
        div()
            .size(px(14.0))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded_sm()
            .border_1()
            .border_color(rgb(color))
            .when(checked, |d| d.bg(rgb(color)))
            .when(checked, |d| {
                d.child(
                    div()
                        .text_size(px(10.0))
                        .font_weight(FontWeight::BOLD)
                        .text_color(rgb(ON_ACCENT))
                        .child("✓"),
                )
            }),
    )
    .child(
        div()
            .flex_1()
            .min_w(px(0.0))
            .truncate()
            .text_size(px(12.0))
            .text_color(rgb(if checked { TEXT } else { MUTED }))
            .child(label.into()),
    )
}

/// A calendar's visibility toggle: a filled dot when shown, a hollow ring when hidden.
pub fn calendar_toggle_row(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    color: u32,
    checked: bool,
    on_click: Option<OnClick>,
) -> Stateful<Div> {
    let row = div()
        .id(id)
        .w_full()
        .h(px(CAL_ROW_H))
        .flex()
        .items_center()
        .gap(px(10.0))
        .rounded_md()
        .px(px(8.0));
    let row = match on_click {
        Some(handler) => row
            .hover(|s| s.bg(rgb(HOVER)).cursor_pointer())
            .on_click(handler),
        None => row,
    };
    row.child(
        div()
            .size(px(CAL_DOT))
            .flex_shrink_0()
            .rounded_full()
            .border(px(1.5))
            .border_color(rgb(color))
            .when(checked, |d| d.bg(rgb(color))),
    )
    .child(
        div()
            .flex_1()
            .min_w(px(0.0))
            .truncate()
            .text_size(px(12.5))
            .font_weight(FontWeight::NORMAL)
            .text_color(rgb(if checked { TEXT } else { MUTED }))
            .child(label.into()),
    )
}

pub fn card(title: impl Into<SharedString>) -> Div {
    div()
        .w_full()
        .flex()
        .flex_col()
        .gap_3()
        .rounded_lg()
        .border_1()
        .border_color(rgb(BORDER))
        .bg(rgb(PANEL))
        .p_5()
        .child(
            div()
                .text_size(px(14.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(rgb(TEXT))
                .child(title.into()),
        )
}

pub fn body_text(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(12.0))
        .line_height(px(18.0))
        .text_color(rgb(SECONDARY))
        .child(text.into())
}

/// Ends a row whose text was cut short.
const ELLIPSIS: &str = "…";
/// Slack for float error when comparing shaped widths and row heights.
const FIT_EPS: f32 = 0.01;

/// Title rows and whether the time row shows, for a timed block whose text area is
/// `content_h` tall. The time needs room for one title row above it.
pub fn event_rows(content_h: f32) -> (usize, bool) {
    let show_time = content_h + FIT_EPS >= EVENT_LINE_H + EVENT_META_LINE_H;
    (title_row_budget(content_h, show_time), show_time)
}

/// Whole title rows that fit in `content_h`, less the time row when it shows; never 0.
fn title_row_budget(content_h: f32, with_time: bool) -> usize {
    let free = content_h + FIT_EPS - if with_time { EVENT_META_LINE_H } else { 0.0 };
    ((free / EVENT_LINE_H).floor() as usize).max(1)
}

/// Byte ranges of the whitespace-separated words in `text`.
fn words(text: &str) -> Vec<Range<usize>> {
    let mut words = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                words.push(s..i);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        words.push(s..text.len());
    }
    words
}

/// `text[start..end]` less trailing whitespace and ASCII punctuation, keeping its first char,
/// so a cut never reads "Pairing:…".
fn trim_cut(text: &str, start: usize, end: usize) -> usize {
    let row = &text[start..end];
    let first = row.chars().next().map_or(0, char::len_utf8);
    let kept = row
        .trim_end_matches(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
        .len();
    start + kept.max(first)
}

/// Splits `text` into at most `max_rows` rows, breaking only between words. `x_for` is the x of
/// a byte index in the unwrapped shaped text. The last row allowed, or a row whose first word
/// is too wide alone, takes the rest and is cut with "…" (the flag) where it overflows. A row
/// that would hold only "…" is dropped.
pub fn title_rows(
    text: &str,
    x_for: impl Fn(usize) -> f32,
    width: f32,
    max_rows: usize,
    ellipsis_w: f32,
) -> Vec<(Range<usize>, bool)> {
    let words = words(text);
    let mut rows = Vec::new();
    let mut i = 0;
    while i < words.len() && rows.len() < max_rows {
        let start = words[i].start;
        let fits = |end: usize| x_for(end) - x_for(start) <= width + FIT_EPS;
        if rows.len() + 1 < max_rows && fits(words[i].end) {
            let mut j = i;
            while j + 1 < words.len() && fits(words[j + 1].end) {
                j += 1;
            }
            rows.push((start..words[j].end, false));
            i = j + 1;
            continue;
        }
        let end = words[words.len() - 1].end;
        if fits(end) {
            rows.push((start..end, false));
            break;
        }
        let mut cut = None;
        for (idx, c) in text[start..end].char_indices() {
            let k = start + idx + c.len_utf8();
            if x_for(k) - x_for(start) + ellipsis_w > width + FIT_EPS {
                break;
            }
            cut = Some(k);
        }
        if let Some(k) = cut {
            rows.push((start..trim_cut(text, start, k), true));
        }
        break;
    }
    rows
}

/// The first of `widths` no wider than `width`. A time is never cut: a partial "10:2…"
/// reads as a real time.
fn first_fitting(widths: impl IntoIterator<Item = f32>, width: f32) -> Option<usize> {
    widths.into_iter().position(|w| w <= width + FIT_EPS)
}

/// Shapes `text[range]` (with "…" when `ellipsis`) via `shape`, which returns the shaped row and
/// its width. Shaping a row alone can come out wider than the unwrapped measure (kerning,
/// ligatures), so it drops a char at a time, cut with "…", until the row fits. Returns the row
/// and whether it ends in "…", or None if not even one char fits.
fn fit_row<T>(
    text: &str,
    range: Range<usize>,
    mut ellipsis: bool,
    width: f32,
    mut shape: impl FnMut(String) -> (T, f32),
) -> Option<(T, bool)> {
    let start = range.start;
    let mut end = range.end;
    while end > start {
        let row = &text[start..end];
        let shown = if ellipsis {
            format!("{row}{ELLIPSIS}")
        } else {
            row.to_string()
        };
        let (line, w) = shape(shown);
        if w <= width + FIT_EPS {
            return Some((line, ellipsis));
        }
        let Some((last, _)) = row.char_indices().next_back() else {
            break;
        };
        if last == 0 {
            break;
        }
        ellipsis = true;
        end = trim_cut(text, start, start + last);
    }
    None
}

/// The text of a timed week or day block: whole-word title rows, one time row, then the
/// location, each drawn only where it fits whole. It lays out its own rows because gpui's
/// wrapper splits words by the char and can leave a blank row. Fills its parent's content box.
pub struct EventText {
    title: SharedString,
    title_color: Hsla,
    declined: bool,
    /// Widest first; the first that fits is drawn.
    times: Vec<SharedString>,
    location: Option<SharedString>,
    meta_color: Hsla,
    max_title_rows: Option<usize>,
}

impl EventText {
    pub fn new(
        title: impl Into<SharedString>,
        title_color: u32,
        declined: bool,
        times: Vec<SharedString>,
        location: Option<SharedString>,
        meta_color: u32,
        max_title_rows: Option<usize>,
    ) -> Self {
        Self {
            title: title.into(),
            title_color: rgb(title_color).into(),
            declined,
            times,
            location,
            meta_color: rgb(meta_color).into(),
            max_title_rows,
        }
    }
}

impl IntoElement for EventText {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for EventText {
    type RequestLayoutState = ();
    /// Shaped rows with their line heights, top to bottom.
    type PrepaintState = Vec<(ShapedLine, f32)>;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        style.min_size.width = px(0.).into();
        style.min_size.height = px(0.).into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
        let width = f32::from(bounds.size.width);
        let height = f32::from(bounds.size.height);
        let base = window.text_style().font();
        let title_font = Font {
            weight: FontWeight::SEMIBOLD,
            ..base.clone()
        };
        let time_font = Font {
            weight: FontWeight::MEDIUM,
            features: tabular(),
            ..base.clone()
        };
        let text_system = window.text_system().clone();
        // The same 1px line as a div's `line_through()`.
        let strike = self.declined.then(|| StrikethroughStyle {
            thickness: px(1.),
            ..Default::default()
        });
        let shape_run = |text: SharedString,
                         font: &Font,
                         size: f32,
                         color: Hsla,
                         strikethrough: Option<StrikethroughStyle>| {
            let run = TextRun {
                len: text.len(),
                font: font.clone(),
                color,
                background_color: None,
                underline: None,
                strikethrough,
            };
            let line = text_system.shape_line(text, px(size), &[run], None);
            let w = f32::from(line.width);
            (line, w)
        };
        let shape = |text, font: &Font, size, color| shape_run(text, font, size, color, None);
        // Declined: only the title is struck, as before; the time and location stay plain.
        let shape_title =
            |text| shape_run(text, &title_font, EVENT_TITLE_PX, self.title_color, strike);

        // The time first: if no form fits, its row goes to the title.
        let (mut budget, show_time) = event_rows(height);
        let time = show_time
            .then(|| {
                let shaped: Vec<_> = self
                    .times
                    .iter()
                    .map(|t| shape(t.clone(), &time_font, EVENT_META_PX, self.meta_color))
                    .collect();
                let i = first_fitting(shaped.iter().map(|(_, w)| *w), width)?;
                shaped.into_iter().nth(i).map(|(line, _)| line)
            })
            .flatten();
        if show_time && time.is_none() {
            budget = title_row_budget(height, false);
        }
        if let Some(max) = self.max_title_rows {
            budget = budget.min(max);
        }

        let mut rows = Vec::new();
        let title = self.title.as_ref();
        let (full, _) = shape_title(self.title.clone());
        let (_, ellipsis_w) = shape_title(ELLIPSIS.into());
        let breaks = title_rows(
            title,
            |i| f32::from(full.x_for_index(i)),
            width,
            budget,
            ellipsis_w,
        );
        let mut whole = !breaks.is_empty();
        for (range, ellipsis) in breaks {
            let fitted = fit_row(title, range, ellipsis, width, |s| shape_title(s.into()));
            let Some((line, cut)) = fitted else {
                whole = false;
                break;
            };
            rows.push((line, EVENT_LINE_H));
            if cut {
                whole = false;
                break;
            }
        }
        if let Some(time) = time {
            rows.push((time, EVENT_META_LINE_H));
        }

        // Only under a whole title, and only with a full row left.
        let used: f32 = rows.iter().map(|(_, h)| h).sum();
        if let Some(location) = self.location.as_ref()
            && whole
            && height + FIT_EPS - used >= EVENT_META_LINE_H
        {
            let (full, _) = shape(location.clone(), &base, EVENT_META_PX, self.meta_color);
            let (_, ellipsis_w) = shape(ELLIPSIS.into(), &base, EVENT_META_PX, self.meta_color);
            let breaks = title_rows(
                location,
                |i| f32::from(full.x_for_index(i)),
                width,
                1,
                ellipsis_w,
            );
            if let Some((range, ellipsis)) = breaks.into_iter().next()
                && let Some((line, _)) = fit_row(location, range, ellipsis, width, |s| {
                    shape(s.into(), &base, EVENT_META_PX, self.meta_color)
                })
            {
                rows.push((line, EVENT_META_LINE_H));
            }
        }
        rows
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        rows: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let mut y = bounds.top();
        for (line, line_h) in rows.iter() {
            if let Err(error) = line.paint(
                point(bounds.left(), y),
                px(*line_h),
                TextAlign::Left,
                None,
                window,
                cx,
            ) {
                eprintln!("antarctical: could not paint event text: {error}");
                return;
            }
            y += px(*line_h);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 6px per char, as a monospace stand-in for shaping.
    fn rows(text: &str, width: f32, max_rows: usize) -> Vec<String> {
        let x_for = |i: usize| text[..i].chars().count() as f32 * 6.0;
        title_rows(text, x_for, width, max_rows, 6.0)
            .into_iter()
            .map(|(r, ellipsis)| {
                let row = &text[r];
                if ellipsis {
                    format!("{row}{ELLIPSIS}")
                } else {
                    row.to_string()
                }
            })
            .collect()
    }

    #[test]
    fn titles_wrap_only_at_words() {
        let r = rows("Design review: sync engine", 42.0, 3);
        assert_eq!(r, ["Design", "review:", "sync e…"]);
        assert!(r.iter().all(|row| !row.trim().is_empty()));
        assert_eq!(rows("Don't panic", 30.0, 2), ["Don't", "panic"]);
        assert_eq!(rows("Supercalifragilistic", 42.0, 3), ["Superc…"]);
        assert_eq!(rows("Pairing: tray agenda", 54.0, 1), ["Pairing…"]);
        assert_eq!(rows("Pairing: tray agenda", 30.0, 1), ["Pair…"]);
        assert_eq!(rows("a/b/c/d/e/f/g", 30.0, 3), ["a/b…"]);
        assert_eq!(rows("  Design   review  ", 42.0, 3), ["Design", "review"]);
        assert!(rows("Design review", 8.0, 3).is_empty());
        assert!(rows("   ", 42.0, 3).is_empty());
        assert!(rows("Design", 42.0, 0).is_empty());
    }

    #[test]
    fn row_budget_fits_the_block() {
        assert_eq!(event_rows(14.0), (1, false));
        assert_eq!(event_rows(20.0), (1, false));
        assert_eq!(event_rows(26.0), (1, false));
        assert_eq!(event_rows(27.0), (1, true));
        assert_eq!(event_rows(42.0), (2, true));
        assert_eq!(event_rows(66.0), (3, true));
        for step in 28..=800 {
            let h = step as f32 * 0.5;
            let (rows, time) = event_rows(h);
            let used = rows as f32 * EVENT_LINE_H + if time { EVENT_META_LINE_H } else { 0.0 };
            assert!(used <= h, "{h}: {rows} rows, time {time}");
            // With no time that fits, the title takes the freed row.
            assert!(title_row_budget(h, false) as f32 * EVENT_LINE_H <= h, "{h}");
        }
    }

    #[test]
    fn time_is_whole_or_absent() {
        let widths = ["10:26am – 11:11am", "10:26am"].map(|t| t.chars().count() as f32 * 6.0);
        assert_eq!(first_fitting(widths, 102.0), Some(0));
        assert_eq!(first_fitting(widths, 60.0), Some(1));
        assert_eq!(first_fitting(widths, 30.0), None);
        assert_eq!(first_fitting(widths, 8.0), None);
        assert_eq!(first_fitting([], 102.0), None);
    }

    #[test]
    fn reshaped_rows_never_overflow() {
        // Rows of 4+ chars shape 5px wider than their chars, as kerning or a ligature might.
        let shape = |s: String| {
            let n = s.chars().count() as f32;
            let w = n * 6.0 + if n >= 4.0 { 5.0 } else { 0.0 };
            (s, w)
        };
        let text = "Standup notes";
        let fit = |r: Range<usize>, e: bool, w: f32| fit_row(text, r, e, w, shape);
        assert_eq!(fit(0..7, false, 60.0), Some(("Standup".into(), false)));
        // 42px fits "Standup" by the measure; shaped alone it is 47px, so it is cut.
        assert_eq!(fit(0..7, false, 42.0), Some(("Stand…".into(), true)));
        assert_eq!(fit(0..7, true, 30.0), Some(("Sta…".into(), true)));
        assert_eq!(fit(0..7, true, 8.0), None);
        for w in [8.0, 13.0, 20.0, 31.0, 42.0, 55.0] {
            if let Some((row, _)) = fit(0..13, true, w) {
                assert!(shape(row).1 <= w, "{w}");
            }
        }
    }
}
