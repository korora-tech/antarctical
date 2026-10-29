//! Stateless building blocks. Disabled controls never get an `on_click`.

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Div, ElementId, FontWeight, Hsla, SharedString, Stateful, Window, div, px,
    rgb, transparent_black,
};

use super::theme::{
    ACCENT, ACCENT_TEXT, BORDER, CAL_DOT, CAL_ROW_H, CAPTION, CONTROL_H, DISABLED, HOVER, MUTED,
    ON_ACCENT, PANEL, PANEL_RAISED, RED, SECONDARY, SEGMENT_ACTIVE, SEGMENT_ACTIVE_BORDER,
    SEGMENT_HOVER, SEGMENT_TRACK, TEXT,
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
