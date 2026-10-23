//! Colour tokens and sizes, shared with chinstrap's dark palette.

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
pub const CAPTION: u32 = 0x858c9c;
pub const SECONDARY: u32 = 0xc3c6d1;
pub const DISABLED: u32 = 0x626878;
pub const ACCENT_TEXT: u32 = 0xd9d0ff;
/// Days outside the current month in the month grid.
pub const OUTSIDE: u32 = 0x0f1118;
pub const NOW_LINE: u32 = 0xff6b7a;

pub const HOUR_PX: f32 = 48.0;
pub const SIDEBAR_W: f32 = 256.0;
pub const DETAIL_W: f32 = 352.0;

/// Mixes `color` into `base` by `amount` (0–1): event chips are tinted, not saturated.
pub fn tint(color: u32, base: u32, amount: f32) -> u32 {
    let ch = |c: u32, shift: u32| ((c >> shift) & 0xff) as f32;
    let mix = |shift| {
        let v = ch(base, shift) + (ch(color, shift) - ch(base, shift)) * amount;
        (v.round().clamp(0.0, 255.0) as u32) << shift
    };
    mix(16) | mix(8) | mix(0)
}
