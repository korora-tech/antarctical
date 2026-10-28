//! GPUI front end: full calendar and tray agenda windows over the shared [`Store`].

pub mod calendar_views;
pub mod format;
pub mod main_view;
pub mod settings;
pub mod text_input;
pub mod theme;
pub mod tray_view;
pub mod widgets;

use gpui::{
    App, AppContext, Bounds, Entity, KeyBinding, TitlebarOptions, WindowBounds, WindowOptions,
    actions, px, size,
};
use jiff::civil::Date;

use crate::store::Store;
use crate::sync::{Command, ViewId};

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

pub type TrayWindow = tray_view::TrayWindow;

/// Brings the main calendar forward, opening it if it was closed to the tray.
pub fn show_main_window(store: &Entity<Store>, cx: &mut App) {
    if let Some(window) = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<main_view::MainView>())
    {
        if let Err(error) = window.update(cx, |_, window, _| window.activate_window()) {
            eprintln!("antarctical: could not activate the calendar window: {error}");
        }
        cx.activate(true);
        return;
    }
    show_main_window_at(store, jiff::Zoned::now().date(), cx);
}

/// Brings the calendar forward at a date selected in the tray window.
pub fn show_main_window_at(store: &Entity<Store>, date: Date, cx: &mut App) {
    if let Some(window) = cx
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<main_view::MainView>())
    {
        if let Err(error) = window.update(cx, |view, window, cx| {
            view.focus_date(date, window, cx);
            window.activate_window();
        }) {
            eprintln!("antarctical: could not focus the calendar window on {date}: {error}");
        }
        cx.activate(true);
        return;
    }
    let bounds = Bounds::centered(None, size(px(1280.), px(820.)), cx);
    let store_for_window = store.clone();
    let window = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("Antarctical".into()),
                ..Default::default()
            }),
            app_id: Some("korora.tech.Antarctical".into()),
            window_min_size: Some(size(px(960.), px(620.))),
            ..Default::default()
        },
        move |window, cx| cx.new(|cx| main_view::MainView::new(store_for_window, window, cx)),
    );
    match window {
        Ok(window) => {
            let id = window.window_id();
            let store = store.clone();
            cx.on_window_closed(move |cx, closed| {
                if closed == id {
                    store.read(cx).send(Command::Forget(ViewId::Main));
                }
            })
            .detach();
            if let Err(error) = window.update(cx, |view, window, cx| {
                view.focus_date(date, window, cx);
            }) {
                eprintln!(
                    "antarctical: could not focus the new calendar window on {date}: {error}"
                );
            }
            cx.activate(true);
        }
        Err(e) => eprintln!("antarctical: could not open a window: {e}"),
    }
}

pub fn toggle_tray_window(
    store: &Entity<Store>,
    handle: &TrayWindow,
    click: Option<(i32, i32)>,
    cx: &mut App,
) {
    tray_view::toggle(store, handle, click, cx);
}

pub fn close_tray_window(store: &Entity<Store>, handle: &TrayWindow, cx: &mut App) {
    tray_view::close(store, handle, cx);
}
