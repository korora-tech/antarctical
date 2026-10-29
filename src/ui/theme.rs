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
/// An inactive segment under the pointer, the exact midpoint of track and active
/// (= tint(SEGMENT_ACTIVE, SEGMENT_TRACK, 0.5)) so hover never reads as selected.
pub const SEGMENT_HOVER: u32 = 0x1e212c;
/// Opacity for events that have already ended, like Fantastical's past events. gpui fades each
/// primitive, so text composites as tint(fg, surface, 0.6), and on a filled block or ribbon the
/// surface is itself tint(fill, underlay, 0.6); figures assume sRGB blending, per tint().
/// Flat BG/PANEL: TEXT 6.57/6.31, SECONDARY 4.59/4.43, MUTED 3.34/3.24, CAPTION 2.77/2.66 (hence
/// PAST_CAPTION); on TODAY_BG MUTED 3.19, CAPTION 2.65. Faded week/day block, worst 0x9fe1e7 on
/// TODAY_BG: TEXT 5.44, SECONDARY 3.86, MUTED 2.84 (hence plain SECONDARY for the time and location). Faded ribbon, worst 0x9fe1e7 on
/// PANEL: TEXT 4.88. 0.5 took MUTED to 2.69:1 on BG.
pub const PAST_OPACITY: f32 = 0.6;
/// Opacity for a past event under the pointer.
pub const PAST_HOVER_OPACITY: f32 = 0.9;
/// The segmented RSVP control while a reply is in flight. Equal to PAST_OPACITY in value but not
/// in meaning (an interaction in progress, not elapsed time), so the two can change independently.
pub const RSVP_PENDING_OPACITY: f32 = 0.6;
/// CAPTION's stand-in inside a faded chip: CAPTION fades to 2.66:1 on PANEL, MUTED to 3.24:1.
/// Holds only while MUTED keeps its current value.
pub const PAST_CAPTION: u32 = MUTED;
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
/// Wash behind today's month cell, week column and all-day cell (tint(ACCENT, BG, 0.08)): 1.117:1
/// vs BG. GRID_LINE softens to 1.156:1 on it (1.292:1 on BG), so today's hour lines are fainter.
pub const TODAY_BG: u32 = 0x1d1d2d;
/// Today's week header cell (tint(ACCENT, BG, 0.12)): 1.192:1 vs BG; ACCENT_TEXT on it is 10.7:1.
pub const TODAY_HEADER_BG: u32 = 0x222236;
/// Hover on today's month cell and week header; lighter than both today washes (tint(ACCENT, BG, 0.16)):
/// 1.271:1 vs BG, 1.137:1 vs TODAY_BG. Matches SELECTED (1.001:1), which never shares an element.
pub const TODAY_HOVER: u32 = 0x28263f;
/// Saturday and Sunday header, all-day cell and column in the week grid: a deliberate step below
/// BG (1.080:1, CIE ΔL* 3.8), and the week grid's counterpart of OUTSIDE, so month weekends stay
/// unwashed. The darkest surface; not a recessed-control tone. GRID_LINE on it is 1.395:1.
pub const WEEKEND_BG: u32 = 0x07080c;
pub const NOW_LINE: u32 = 0xff6b7a;
/// The now line on days other than today (= tint(NOW_LINE, BG, 0.30)).
pub const NOW_LINE_FAINT: u32 = 0x582d38;
/// Text on the now pill in the time gutter.
pub const NOW_PILL_TEXT: u32 = 0x1a0d10;
pub const NOW_PILL_H: f32 = 16.0;
/// Hour labels closer than this to the now line are hidden, so the pill never touches one.
pub const NOW_PILL_CLEARANCE: f32 = 18.0;
/// The now line's head. Its 2px BG ring sits inside the 12px, so the red core is 8px; the ring is
/// the halo that sets the dot apart from event blocks.
pub const NOW_DOT: f32 = 12.0;

pub const HOUR_PX: f32 = 48.0;
/// The hour label's line box, centred vertically on its hour line.
pub const HOUR_LABEL_LINE_H: f32 = 12.0;
/// Side padding of the agenda list (the old px_6).
pub const AGENDA_PAD_X: f32 = 24.0;
/// Side padding of the agenda list when the date gutter is stacked.
pub const AGENDA_PAD_X_STACKED: f32 = 16.0;
/// Narrowest pane that keeps the agenda's date gutter: 480 - (48 pad + 72 gutter + 16 gap + 2 border + 28 row pad + 84 time + 24 gaps + 3 bar = 277) leaves the title 203px; narrower panes stack the date above the card.
pub const AGENDA_GUTTER_MIN_W: f32 = 480.0;
/// Shared line box of the stacked agenda day header's runs (13px text).
pub const AGENDA_HEADER_LINE_H: f32 = 18.0;
pub const AGENDA_ROW_H: f32 = 52.0;
/// The agenda's date gutter: fits 'Tomorrow' at 11px semibold plus 6px padding each side.
pub const AGENDA_GUTTER_W: f32 = 72.0;
/// The agenda's time column: fits '12:30pm' at 13px.
pub const AGENDA_TIME_W: f32 = 84.0;
/// Space between agenda day sections.
pub const AGENDA_SECTION_GAP: f32 = 28.0;
/// Corner radius of an agenda day's grouped rows.
pub const AGENDA_RADIUS: f32 = 10.0;
/// The agenda gutter's day numeral.
pub const AGENDA_DAY_PX: f32 = 26.0;
/// Line height of the agenda gutter's day numeral.
pub const AGENDA_DAY_LINE_H: f32 = 30.0;
pub const SIDEBAR_W: f32 = 256.0;
pub const DETAIL_W: f32 = 352.0;
/// Calendar-colour wash behind the detail header; lower for a declined event.
pub const DETAIL_HEADER_TINT: f32 = 0.12;
pub const DETAIL_HEADER_TINT_DECLINED: f32 = 0.05;
/// Glyph column of a detail fact row.
pub const DETAIL_ICON_W: f32 = 18.0;
/// Line height of a detail fact row's text; its icon is centred in a box this tall so it sits
/// on the first line.
pub const DETAIL_LINE_H: f32 = 19.0;
/// Detail fact icons, drawn from divs so they need no font coverage. CAPTION on PANEL is 5.04:1,
/// clear of the 3:1 non-text minimum. Location: a ring this wide with a DETAIL_ICON_STROKE border.
pub const DETAIL_ICON_PX: f32 = 10.0;
pub const DETAIL_ICON_STROKE: f32 = 2.0;
/// Organiser: a filled disc.
pub const DETAIL_ICON_DOT: f32 = 6.0;
/// Description: three bars stacked with a gap between them.
pub const DETAIL_ICON_BAR_W: f32 = 10.0;
pub const DETAIL_ICON_BAR_H: f32 = 2.0;
pub const DETAIL_ICON_BAR_GAP: f32 = 2.0;
pub const DETAIL_ACTION_H: f32 = 32.0;
/// Full-bleed calendar-colour strip at the top of the detail header.
pub const DETAIL_STRIP_H: f32 = 3.0;
/// Diameter of a calendar's on/off dot in the sidebar list.
pub const CAL_DOT: f32 = 10.0;
/// Height of one sidebar calendar row (also the footer's two-line status block).
pub const CAL_ROW_H: f32 = 28.0;
/// Line height of the sidebar footer's two text lines; two of them fill one CAL_ROW_H.
/// Set explicitly: gpui's phi() default at 11px would overflow the block.
pub const FOOTER_LINE_H: f32 = CAL_ROW_H / 2.0;
/// Diameter of the sidebar footer's status dot.
pub const FOOTER_DOT: f32 = 6.0;
/// Gap between the footer's status dot and its label.
pub const FOOTER_DOT_GAP: f32 = 8.0;
/// Indent that lines the footer's sync caption up under the status label: the dot plus its gap.
pub const FOOTER_TEXT_INDENT: f32 = FOOTER_DOT + FOOTER_DOT_GAP;
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
/// Window width below which the calendar header goes compact: no wordmark text and a short day
/// title. The longest full title ('Wednesday, 30 September' 2026, ≈358px at 22px Inter) needs
/// ≈1057px with the wordmark; the longest compact one ('28 Dec – 3 Jan' 2026 – 2027, ≈296px)
/// fits at the 960px minimum. Widths are approximate and assume Inter, so recompute this if
/// TITLE_PX, SEGMENT_W or the header padding changes.
pub const HEADER_COMPACT_BELOW: f32 = 1100.0;
/// The detail pane's event title and its line height.
pub const DETAIL_TITLE_PX: f32 = 18.0;
pub const DETAIL_TITLE_LINE_H: f32 = 24.0;
/// The month grid's weekday row.
pub const WEEKDAY_HEADER_H: f32 = 28.0;
/// The month grid's date numbers.
pub const MONTH_DAY_PX: f32 = 12.0;
/// The month numeral's disc: its height and minimum width.
pub const MONTH_DAY_DISC: f32 = 22.0;
/// Every week/day column header numeral.
pub const DAY_HEADER_NUM_PX: f32 = 17.0;
/// Box for every header numeral, filled with ACCENT for today; also the caps line box.
pub const TODAY_DISC: f32 = 28.0;
/// Week/day column header side padding; small enough that the disc fits a 42px column.
pub const DAY_HEADER_PAD: f32 = 5.0;
/// Space between a column header's weekday caps and its numeral.
pub const DAY_HEADER_GAP: f32 = 2.0;
/// How much of the calendar colour is mixed over PANEL for an all-day bar.
pub const ALL_DAY_FILL: f32 = 0.26;
/// The hovered all-day fill, so the ribbon keeps its colour.
pub const ALL_DAY_FILL_HOVER: f32 = 0.34;
/// The all-day fill for an event you declined.
pub const ALL_DAY_FILL_DECLINED: f32 = 0.12;
/// The calendar-colour accent at the head of an all-day bar.
pub const ALL_DAY_BAR_W: f32 = 3.0;
/// Opacity of that accent bar for an event you declined.
pub const DECLINED_BAR_OPACITY: f32 = 0.4;
/// How much of the calendar colour is mixed over BG for a timed block in the week/day grid.
pub const EVENT_FILL: f32 = 0.20;
/// The hovered timed fill.
pub const EVENT_FILL_HOVER: f32 = 0.28;
/// The selected timed fill. Hover leaves it unchanged, so a selected block never brightens.
pub const EVENT_FILL_SELECTED: f32 = 0.32;
/// The timed fill for an event you declined, and its hover.
pub const EVENT_FILL_DECLINED: f32 = 0.08;
pub const EVENT_FILL_DECLINED_HOVER: f32 = 0.14;
/// How much of the calendar colour a declined block's accent bar keeps (mixed over BG). It
/// recedes on purpose (down to 1.38:1 on its fill, Grape); line-through and MUTED carry the state.
pub const EVENT_BAR_DECLINED: f32 = 0.5;
/// How much of the calendar colour a declined event's detail dot keeps (mixed over PANEL). The dot
/// is decorative; line-through and the MUTED strip carry the declined state.
pub const DECLINED_DOT_TINT: f32 = 0.5;
/// The calendar-colour pill inside a timed block.
pub const EVENT_BAR_W: f32 = 3.0;
/// Gap between the bar and the block's outer edge. It clears the 1px selection ring with 2px to spare.
pub const EVENT_BAR_INSET: f32 = 3.0;
/// Horizontal text padding of a timed block, after the bar.
pub const EVENT_PAD_X: f32 = 4.0;
pub const EVENT_RADIUS: f32 = 4.0;
/// Space a timed block keeps from its column's left and right edges.
pub const EVENT_GUTTER_L: f32 = 2.0;
pub const EVENT_GUTTER_R: f32 = 3.0;
/// Space between side-by-side overlapping blocks.
pub const EVENT_OVERLAP_GAP: f32 = 2.0;
/// Timed block title, and the line height shared by its compact one-line row.
pub const EVENT_TITLE_PX: f32 = 11.5;
pub const EVENT_LINE_H: f32 = 14.0;
/// The time and location lines of a timed block.
pub const EVENT_META_PX: f32 = 10.0;
pub const EVENT_META_LINE_H: f32 = 13.0;
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

/// A timed block's title: mostly TEXT, faintly tinted with the calendar colour.
pub fn event_text(color: u32) -> u32 {
    tint(color, TEXT, 0.25)
}

/// A timed block's time and location: SECONDARY, faintly tinted with the calendar colour.
pub fn event_subtext(color: u32) -> u32 {
    tint(color, SECONDARY, 0.25)
}

/// The selection ring around a timed block. It replaces the ACCENT ring, so the calendar colour stays.
pub fn event_ring(color: u32) -> u32 {
    tint(color, TEXT, 0.35)
}

/// Tabular figures, so date numbers line up in columns.
pub fn tabular() -> FontFeatures {
    FontFeatures(Arc::new(vec![("tnum".into(), 1)]))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WCAG contrast ratio between two colours.
    fn contrast(a: u32, b: u32) -> f32 {
        let lum = |c: u32| {
            let ch = |shift: u32| {
                let v = ((c >> shift) & 0xff) as f32 / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * ch(16) + 0.7152 * ch(8) + 0.0722 * ch(0)
        };
        let (la, lb) = (lum(a), lum(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    /// Faded `fg` over `surface`, as gpui composites each primitive.
    fn faded(fg: u32, surface: u32) -> f32 {
        contrast(tint(fg, surface, PAST_OPACITY), surface)
    }

    #[test]
    fn past_chip_time_stays_legible() {
        for surface in [BG, PANEL, TODAY_BG, WEEKEND_BG, OUTSIDE] {
            assert!(faded(PAST_CAPTION, surface) >= 3.0, "{surface:06x}");
        }
    }

    #[test]
    fn today_washes_match_their_formulas() {
        assert_eq!(TODAY_BG, tint(ACCENT, BG, 0.08));
        assert_eq!(TODAY_HEADER_BG, tint(ACCENT, BG, 0.12));
        assert_eq!(TODAY_HOVER, tint(ACCENT, BG, 0.16));
    }

    #[test]
    fn segment_hover_is_track_active_midpoint() {
        let mid = tint(SEGMENT_ACTIVE, SEGMENT_TRACK, 0.5);
        assert_eq!(mid, SEGMENT_HOVER, "{mid:06x} != {SEGMENT_HOVER:06x}");
    }

    #[test]
    fn past_block_meta_stays_legible() {
        // The lightest calendar colour, on the lightest block underlay.
        let fill = tint(tint(0x9fe1e7, BG, EVENT_FILL), TODAY_BG, PAST_OPACITY);
        assert!(faded(SECONDARY, fill) >= 3.0);
        assert!(faded(TEXT, fill) >= 4.5);
    }

    #[test]
    fn sidebar_footer_text_meets_aa() {
        assert!(contrast(SECONDARY, SIDEBAR) >= 4.5);
        assert!(contrast(CAPTION, SIDEBAR) >= 4.5);
    }
}
