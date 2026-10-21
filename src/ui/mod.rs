//! GPUI front end: one main window (calendar views + settings) over the shared [`Store`].

pub mod calendar_views;
pub mod format;
pub mod main_view;
pub mod settings;
pub mod text_input;
pub mod theme;
pub mod widgets;

use gpui::{
    App, AppContext, Bounds, Entity, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions,
    actions, px, size,
};

use crate::store::Store;

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
        KeyBinding::new("escape", Back, None),
        KeyBinding::new("ctrl-,", OpenSettings, None),
        KeyBinding::new("ctrl-r", Refresh, None),
        KeyBinding::new("ctrl-w", CloseWindow, None),
        KeyBinding::new("ctrl-q", Quit, None),
    ]);
}

/// Brings the main window forward, opening it if it was closed to the tray.
pub fn show_main_window(store: &Entity<Store>, cx: &mut App) {
    if let Some(window) = cx.windows().into_iter().next() {
        let _ = window.update(cx, |_, window, _| window.activate_window());
        return;
    }
    let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
    let store = store.clone();
    let window = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Antarctical".into()),
                ..Default::default()
            }),
            app_id: Some("com.korora.Antarctical".into()),
            window_min_size: Some(size(px(760.), px(480.))),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| main_view::MainView::new(store, window, cx)),
    );
    match window {
        Ok(window) => {
            let _ = window.update(cx, |view, window, cx| view.focus_calendar(window, cx));
            cx.activate(true);
        }
        Err(e) => eprintln!("antarctical: could not open a window: {e}"),
    }
}
