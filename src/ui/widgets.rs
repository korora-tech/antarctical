//! Stateless building blocks. Disabled controls never get an `on_click`.

use gpui::{
    App, ClickEvent, Div, ElementId, FontWeight, SharedString, Stateful, Window, div, prelude::*,
    px, rgb,
};

use super::theme::*;

pub type OnClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ButtonStyle {
    Primary,
    Danger,
    Secondary,
}

/// A pill button. `on_click: None` renders it disabled.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    style: ButtonStyle,
    on_click: Option<OnClick>,
) -> Stateful<Div> {
    let enabled = on_click.is_some();
    let (bg, fg, border) = match (style, enabled) {
        (_, false) => (PANEL, DISABLED, BORDER),
        (ButtonStyle::Primary, true) => (ACCENT, 0x11111a, ACCENT),
        (ButtonStyle::Danger, true) => (PANEL_RAISED, RED, 0x5a2a32),
        (ButtonStyle::Secondary, true) => (PANEL_RAISED, TEXT, 0x303646),
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
        .bg(rgb(bg))
        .px_3()
        .py(px(5.0))
        .text_size(px(12.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(fg))
        .child(label.into());
    match on_click {
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

/// One option of a segmented control.
pub fn segment(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
    on_click: OnClick,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .rounded_md()
        .px(px(11.0))
        .py(px(6.0))
        .bg(rgb(if active { SELECTED } else { HEADER }))
        .hover(|s| {
            s.bg(rgb(if active { SELECTED } else { HOVER }))
                .cursor_pointer()
        })
        .on_click(on_click)
        .child(
            div()
                .text_size(px(11.0))
                .font_weight(if active {
                    FontWeight::SEMIBOLD
                } else {
                    FontWeight::MEDIUM
                })
                .text_color(rgb(if active { TEXT } else { SECONDARY }))
                .child(label.into()),
        )
}

pub fn caption(text: impl Into<SharedString>) -> Div {
    div()
        .text_size(px(10.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(CAPTION))
        .child(text.into())
}

/// A checkbox tinted with `color` (calendar toggles, preferences).
pub fn checkbox_row(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    color: u32,
    checked: bool,
    on_click: OnClick,
) -> Stateful<Div> {
    div()
        .id(id)
        .w_full()
        .flex()
        .items_center()
        .gap_2()
        .rounded_md()
        .px_2()
        .py(px(5.0))
        .hover(|s| s.bg(rgb(HOVER)).cursor_pointer())
        .on_click(on_click)
        .child(
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
                            .text_color(rgb(0x11111a))
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
