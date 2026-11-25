//! GPUI front end: full calendar and tray agenda windows over the shared [`crate::store::Store`].

pub mod calendar_views;
pub mod compose;
pub mod format;
pub mod import;
pub mod main_view;
pub mod palette;
pub mod settings;
pub mod text_input;
pub mod theme;
pub mod tray_view;
pub mod widgets;
pub mod windows;

use gpui::{App, KeyBinding, actions};

actions!(
    antarctical,
    [
        Today,
        Previous,
        Next,
        ShowMonth,
        ShowWeek,
        ShowDay,
        ShowAgenda,
        OpenSettings,
        Back,
        Refresh,
        CloseWindow,
        Quit,
        NewEvent,
        EditEvent,
        OpenPalette,
        PaletteUp,
        PaletteDown,
    ]
);

pub fn bind_keys(cx: &mut App) {
    text_input::bind_keys(cx);
    let cal = Some("Calendar");
    cx.bind_keys([
        KeyBinding::new("t", Today, cal),
        KeyBinding::new("left", Previous, cal),
        KeyBinding::new("right", Next, cal),
        KeyBinding::new("h", Previous, cal),
        KeyBinding::new("l", Next, cal),
        KeyBinding::new("m", ShowMonth, cal),
        KeyBinding::new("w", ShowWeek, cal),
        KeyBinding::new("d", ShowDay, cal),
        KeyBinding::new("a", ShowAgenda, cal),
        KeyBinding::new("n", NewEvent, cal),
        KeyBinding::new("e", EditEvent, cal),
        KeyBinding::new("ctrl-n", NewEvent, None),
        KeyBinding::new("ctrl-k", OpenPalette, None),
        KeyBinding::new("up", PaletteUp, Some("Palette")),
        KeyBinding::new("down", PaletteDown, Some("Palette")),
        KeyBinding::new("escape", Back, None),
        KeyBinding::new("ctrl-,", OpenSettings, None),
        KeyBinding::new("ctrl-r", Refresh, None),
        KeyBinding::new("ctrl-w", CloseWindow, None),
        KeyBinding::new("ctrl-q", Quit, None),
    ]);
}
