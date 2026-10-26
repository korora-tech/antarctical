//! Opening, focusing and closing the main calendar window.

use gpui::{
    App, AppContext, Bounds, Entity, TitlebarOptions, WindowBounds, WindowHandle, WindowOptions,
    px, size,
};
use jiff::civil::Date;

use super::main_view;
use crate::branding;
use crate::store::Store;
use crate::sync::{Command, ViewId};

fn main_window(cx: &App) -> Option<WindowHandle<main_view::MainView>> {
    cx.windows()
        .into_iter()
        .find_map(|window| window.downcast::<main_view::MainView>())
}

/// Brings the main calendar forward, opening it if it was closed to the tray.
pub fn show_main_window(store: &Entity<Store>, cx: &mut App) {
    if let Some(window) = main_window(cx) {
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
    if let Some(window) = main_window(cx) {
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
            app_id: Some(branding::APP_ID.into()),
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
