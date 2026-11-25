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

/// Reads an `.ics` file and offers to import its events in the main window.
pub fn import_ics(store: &Entity<Store>, path: &std::path::Path, cx: &mut App) {
    use std::io::Read;
    let name = path.file_name().map_or_else(
        || "calendar file".to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let read = std::fs::File::open(path).and_then(|file| {
        let mut text = String::new();
        file.take(crate::ics::MAX_BYTES + 1)
            .read_to_string(&mut text)?;
        Ok(text)
    });
    let outcome = match read {
        Ok(text) if text.len() as u64 > crate::ics::MAX_BYTES => Err(format!(
            "{name} is larger than 1 MB, which is too big to import."
        )),
        Ok(text) => Ok(crate::ics::parse(
            &text,
            &jiff::tz::TimeZone::system(),
            &crate::zones::lookup,
        )),
        Err(e) => Err(format!("Could not read {name}: {e}")),
    };
    show_main_window(store, cx);
    if let Some(window) = main_window(cx)
        && let Err(error) = window.update(cx, |view, window, cx| {
            view.open_import(name, outcome, window, cx)
        })
    {
        eprintln!("antarctical: could not show the import: {error}");
    }
}
