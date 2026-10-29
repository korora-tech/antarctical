//! Colour tokens and sizes, shared with chinstrap's dark palette.

use std::sync::Arc;

use gpui::FontFeatures;

pub const BG: u32 = 0x11131b;
pub const PANEL: u32 = 0x191c27;
pub const PANEL_RAISED: u32 = 0x222633;
pub const BORDER: u32 = 0x2b2f3d;
pub const GRID_LINE: u32 = 0x252a36;
pub const TEXT: u32 = 0xf5f3fa;
pub const MUTED: u32 = 0x9ba1b1;
pub const ACCENT: u32 = 0xa28cff;
pub const GREEN: u32 = 0x68d9aa;
pub const RED: u32 = 0xff6b7a;
pub const AMBER: u32 = 0xffc36a;

pub const HEADER: u32 = 0x171923;
pub const SIDEBAR: u32 = 0x151721;
pub const SELECTED: u32 = 0x2a263c;
pub const SELECTED_BORDER: u32 = 0x5c4e85;
pub const HOVER: u32 = 0x292d3b;
/// Recessed track behind a segmented control (view switcher, date navigation).
pub const SEGMENT_TRACK: u32 = 0x0d0f15;
/// The raised active segment and its outline.
pub const SEGMENT_ACTIVE: u32 = 0x2e3242;
pub const SEGMENT_ACTIVE_BORDER: u32 = 0x3a3f52;
/// An inactive segment under the pointer, about midway between track and active
/// (tint(SEGMENT_ACTIVE, SEGMENT_TRACK, 0.5)) so hover never reads as selected.
pub const SEGMENT_HOVER: u32 = 0x1d202b;
/// Opacity for events that have already ended. Deliberately de-emphasised: TEXT stays 6.6:1 on
/// BG, but SECONDARY (4.6:1) and MUTED (3.3:1) drop to or below AA for small text, and lower on
/// tinted fills; legible, like Fantastical's past events. 0.5 took MUTED to 2.7:1.
pub const PAST_OPACITY: f32 = 0.6;
/// Opacity for a past event under the pointer.
pub const PAST_HOVER_OPACITY: f32 = 0.9;
pub const CAPTION: u32 = 0x858c9c;
pub const SECONDARY: u32 = 0xc3c6d1;
pub const DISABLED: u32 = 0x626878;
pub const ACCENT_TEXT: u32 = 0xd9d0ff;
/// Dark ink on ACCENT and calendar-colour fills (6.9:1 on ACCENT).
pub const ON_ACCENT: u32 = 0x11111a;
/// Date numbers of in-month days before today; recedes but stays >= 4.5:1 on BG and PANEL.
pub const PAST_DAY_TEXT: u32 = 0x7e8496;
/// Days outside the current month in the month grid.
pub const OUTSIDE: u32 = 0x0f1118;
/// Wash behind today's month cell, week column and all-day cell (tint(ACCENT, BG, 0.05)).
pub const TODAY_BG: u32 = 0x181926;
/// Today's week header cell (tint(ACCENT, BG, 0.09)).
pub const TODAY_HEADER_BG: u32 = 0x1e1e30;
/// Hover on today's month cell and week header; lighter than both today washes (tint(ACCENT, BG, 0.14)).
pub const TODAY_HOVER: u32 = 0x25243b;
/// Saturday and Sunday header, all-day cell and column in the week grid; a visible step below BG.
pub const WEEKEND_BG: u32 = 0x0c0e15;
pub const NOW_LINE: u32 = 0xff6b7a;
/// The now line on days other than today (= tint(NOW_LINE, BG, 0.30)).
pub const NOW_LINE_FAINT: u32 = 0x582d38;
/// Text on the now pill in the time gutter.
pub const NOW_PILL_TEXT: u32 = 0x1a0d10;
pub const NOW_PILL_H: f32 = 16.0;
/// Hour labels closer than this to the now line are hidden, so the pill never touches one.
pub const NOW_PILL_CLEARANCE: f32 = 18.0;

pub const HOUR_PX: f32 = 48.0;
/// The agenda list's centred column.
pub const AGENDA_MAX_W: f32 = 760.0;
pub const AGENDA_ROW_H: f32 = 52.0;
/// The agenda's date gutter: fits 'Tomorrow' at 11px semibold plus 6px padding each side.
pub const AGENDA_GUTTER_W: f32 = 72.0;
/// The agenda's time column: fits '12:30pm' at 13px.
pub const AGENDA_TIME_W: f32 = 84.0;
/// Space between agenda day sections.
pub const AGENDA_SECTION_GAP: f32 = 28.0;
/// Corner radius of an agenda day's grouped rows.
pub const AGENDA_RADIUS: f32 = 10.0;
pub const SIDEBAR_W: f32 = 256.0;
pub const DETAIL_W: f32 = 352.0;
/// Calendar-colour wash behind the detail header; lower for a declined event.
pub const DETAIL_HEADER_TINT: f32 = 0.12;
pub const DETAIL_HEADER_TINT_DECLINED: f32 = 0.05;
/// Glyph column of a detail fact row.
pub const DETAIL_ICON_W: f32 = 18.0;
pub const DETAIL_ACTION_H: f32 = 32.0;
/// Full-bleed calendar-colour strip at the top of the detail header.
pub const DETAIL_STRIP_H: f32 = 3.0;
/// Diameter of a calendar's on/off dot in the sidebar list.
pub const CAL_DOT: f32 = 10.0;
/// Height of one sidebar calendar row (also the footer status row).
pub const CAL_ROW_H: f32 = 28.0;
/// The week and day grid's column header row.
pub const DAY_HEADER_H: f32 = 52.0;
/// The window header (wordmark, title, toolbar).
pub const HEADER_H: f32 = 64.0;
/// The height of every header control (navigation, view switcher, Settings).
pub const CONTROL_H: f32 = 30.0;
/// Fixed view-switcher segment width, so the bold active label never shifts its neighbours.
/// Fits "Agenda" at 12px semibold; longer localised labels would need more.
pub const SEGMENT_W: f32 = 68.0;
/// The header's view title.
pub const TITLE_PX: f32 = 22.0;
/// The month grid's weekday row.
pub const WEEKDAY_HEADER_H: f32 = 28.0;
/// The month grid's date numbers.
pub const MONTH_DAY_PX: f32 = 12.0;
/// How much of the calendar colour is mixed over PANEL for an all-day bar.
pub const ALL_DAY_FILL: f32 = 0.26;
/// The hovered all-day fill, so the ribbon keeps its colour.
pub const ALL_DAY_FILL_HOVER: f32 = 0.34;
/// The all-day fill for an event you declined.
pub const ALL_DAY_FILL_DECLINED: f32 = 0.12;
/// The calendar-colour accent at the head of an all-day bar.
pub const ALL_DAY_BAR_W: f32 = 3.0;
/// The minimum time column of a timed chip in 24h mode ('09:30'), so titles line up.
pub const CHIP_TIME_W_24H: f32 = 32.0;
/// The minimum time column of a timed chip in 12h mode ('10:30pm', measured at 10.5px).
pub const CHIP_TIME_W_12H: f32 = 42.0;

/// Mixes `color` into `base` by `amount` (0–1): event chips are tinted, not saturated.
pub fn tint(color: u32, base: u32, amount: f32) -> u32 {
    let ch = |c: u32, shift: u32| ((c >> shift) & 0xff) as f32;
    let mix = |shift| {
        let v = ch(base, shift) + (ch(color, shift) - ch(base, shift)) * amount;
        (v.round().clamp(0.0, 255.0) as u32) << shift
    };
    mix(16) | mix(8) | mix(0)
}

/// Tabular figures, so date numbers line up in columns.
pub fn tabular() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}
