//! Colour tokens and sizes, shared with chinstrap's dark palette.

pub const BG: u32 = 0x0d0f14;
pub const PANEL: u32 = 0x12151c;
pub const PANEL_RAISED: u32 = 0x171b24;
pub const BORDER: u32 = 0x272d38;
pub const GRID_LINE: u32 = 0x1c2029;
pub const TEXT: u32 = 0xf2f3f6;
pub const MUTED: u32 = 0x8d95a4;
pub const ACCENT: u32 = 0x9c83ff;
pub const GREEN: u32 = 0x63d7a3;
pub const RED: u32 = 0xff5c6c;
pub const AMBER: u32 = 0xffb84d;

pub const HEADER: u32 = 0x11141a;
pub const SIDEBAR: u32 = 0x101319;
pub const SELECTED: u32 = 0x252332;
pub const SELECTED_BORDER: u32 = 0x494260;
pub const HOVER: u32 = 0x20232c;
pub const CAPTION: u32 = 0x717987;
pub const SECONDARY: u32 = 0xb5bdc8;
pub const DISABLED: u32 = 0x4f5661;
pub const ACCENT_TEXT: u32 = 0xd2c8ff;
/// Days outside the current month in the month grid.
pub const OUTSIDE: u32 = 0x0b0d11;
pub const NOW_LINE: u32 = 0xff5c6c;

pub const HOUR_PX: f32 = 48.0;
pub const SIDEBAR_W: f32 = 248.0;
pub const DETAIL_W: f32 = 340.0;

/// Mixes `color` into `base` by `amount` (0–1): event chips are tinted, not saturated.
pub fn tint(color: u32, base: u32, amount: f32) -> u32 {
    let ch = |c: u32, shift: u32| ((c >> shift) & 0xff) as f32;
    let mix = |shift| {
        let v = ch(base, shift) + (ch(color, shift) - ch(base, shift)) * amount;
        (v.round().clamp(0.0, 255.0) as u32) << shift
    };
    mix(16) | mix(8) | mix(0)
}
