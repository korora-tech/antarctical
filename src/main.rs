mod config;
mod demo;
mod google;
mod layout;
mod model;
mod reminders;
mod secrets;
mod store;
mod sync;
mod tray;
mod ui;

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

use gpui::{App, AppContext, Entity, QuitMode};
use gpui_platform::application;
use jiff::tz::TimeZone;
use jiff::{Timestamp, Zoned};

use store::Store;
use sync::Command;
use tray::{Tray, TrayAction, TrayContent};

const USAGE: &str = "usage: antarctical [--demo] [--hidden]

  --demo     run with generated calendars (no Google account, nothing stored)
  --hidden   start in the tray without opening the window

Environment: ANTARCTICAL_DEMO=1 is the same as --demo.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut demo = std::env::var_os("ANTARCTICAL_DEMO").is_some_and(|v| v == "1");
    let mut hidden = false;
    for arg in &args {
        match arg.as_str() {
            "--demo" => demo = true,
            "--hidden" => hidden = true,
            "-h" | "--help" => {
                println!("{USAGE}");
                return;
            }
            "-V" | "--version" => {
                println!("antarctical {}", env!("CARGO_PKG_VERSION"));
                return;
            }
            other => {
                eprintln!("antarctical: unknown argument {other}\n{USAGE}");
                std::process::exit(2);
            }
        }
    }

    let mut prefs = config::Prefs::load();
    prefs.ephemeral = demo;
    let (sync, events) = if demo {
        demo::start()
    } else {
        sync::start_google(secrets::Secrets::keyring())
    };

    // Explicit: closing the last window must not quit while the tray keeps the app alive;
    // `on_window_closed` below quits when there is no tray.
    application()
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx: &mut App| {
            ui::bind_keys(cx);
            let store = cx.new(|_| Store::new(sync.clone(), demo, prefs));

            // Sync worker → store.
            {
                let store = store.clone();
                cx.spawn(async move |cx| {
                    while let Ok(ev) = events.recv().await {
                        store.update(cx, |s, cx| {
                            s.apply(ev);
                            cx.notify();
                        });
                    }
                })
                .detach();
            }

            let (tray_tx, tray_rx) = async_channel::unbounded();
            let tray = match Tray::spawn(tray_content(store.read(cx)), tray_tx) {
                Ok(t) => Some(Rc::new(TrayPush::new(t))),
                Err(e) => {
                    eprintln!("antarctical: no system tray ({e}); closing the window will quit");
                    None
                }
            };
            let has_tray = tray.is_some();

            // Tray menu → app.
            {
                let store = store.clone();
                cx.spawn(async move |cx| {
                    while let Ok(action) = tray_rx.recv().await {
                        cx.update(|cx| match action {
                            TrayAction::Open => ui::show_main_window(&store, cx),
                            TrayAction::Refresh => store.read(cx).send(Command::Refresh),
                            TrayAction::Quit => cx.quit(),
                        });
                    }
                })
                .detach();
            }

            // Tray contents follow the store, and the clock (date rollover, events ending).
            if let Some(tray) = tray.clone() {
                cx.observe(&store, move |store, cx| {
                    tray.push(tray_content(store.read(cx)), cx)
                })
                .detach();
            }
            start_minute_tasks(store.clone(), tray.clone(), cx);

            // With a tray the app lives on after its window closes; without one, it quits.
            cx.on_window_closed(move |cx, _| {
                if cx.windows().is_empty() && !has_tray {
                    cx.quit();
                }
            })
            .detach();
            {
                let sync = sync.clone();
                cx.on_app_quit(move |_| {
                    sync.send(Command::Shutdown);
                    if let Some(tray) = &tray {
                        tray.tray.shutdown();
                    }
                    async {}
                })
                .detach();
            }

            if !(hidden && has_tray) {
                ui::show_main_window(&store, cx);
            }
        });
}

/// Every 20 s: fire due reminders and refresh the tray (date and "what's next" change with time).
fn start_minute_tasks(store: Entity<Store>, tray: Option<Rc<TrayPush>>, cx: &mut App) {
    let fired = Rc::new(RefCell::new(HashSet::<String>::new()));
    cx.spawn(async move |cx| {
        loop {
            cx.update(|cx| {
                let s = store.read(cx);
                if let Some(tray) = &tray {
                    tray.push(tray_content(s), cx);
                }
                if s.prefs.reminders && s.signed_in() {
                    let tz = TimeZone::system();
                    let today = Zoned::now().date();
                    // Google allows popup reminders up to four weeks ahead. Only calendars
                    // shown in the sidebar notify.
                    let events = s.events_between(
                        model::add_days(today, -1),
                        model::add_days(today, 29),
                        &tz,
                    );
                    let due =
                        reminders::due(&events, Timestamp::now(), &tz, &mut fired.borrow_mut());
                    for d in due {
                        // notify-rust blocks on D-Bus; keep it off the UI thread.
                        cx.background_spawn(async move {
                            let mut n = notify_rust::Notification::new();
                            n.appname("Antarctical")
                                .summary(&d.title)
                                .body(&d.body)
                                .icon("x-office-calendar")
                                .timeout(notify_rust::Timeout::Milliseconds(15_000));
                            if let Err(e) = n.show() {
                                eprintln!("antarctical: notification failed: {e}");
                            }
                        })
                        .detach();
                    }
                }
            });
            cx.background_executor()
                .timer(Duration::from_secs(20))
                .await;
        }
    })
    .detach();
}

/// Sends tray content off the UI thread, and only when it changed (most store updates, such
/// as a month of events arriving, leave today's agenda as it was).
struct TrayPush {
    tray: Tray,
    last: RefCell<Option<TrayContent>>,
}

impl TrayPush {
    fn new(tray: Tray) -> Self {
        Self {
            tray,
            last: RefCell::new(None),
        }
    }

    fn push(&self, content: TrayContent, cx: &App) {
        if self.last.borrow().as_ref() == Some(&content) {
            return;
        }
        *self.last.borrow_mut() = Some(content.clone());
        let tray = self.tray.clone();
        cx.background_spawn(async move { tray.update(content) })
            .detach();
    }
}

fn tray_content(store: &Store) -> TrayContent {
    let tz = TimeZone::system();
    let now = Zoned::now();
    let today = now.date();
    let clock = store.prefs.clock_24h;
    let upcoming: Vec<model::Event> = if store.signed_in() {
        store
            .events_on(today, &tz)
            .into_iter()
            .filter(|e| !e.declined() && e.instants(&tz).1 > now.timestamp())
            .collect()
    } else {
        vec![]
    };
    let agenda = upcoming
        .iter()
        .map(|e| {
            let time = if e.is_all_day() {
                "All day".to_string()
            } else {
                ui::format::time(&e.start_zoned(&tz), clock)
            };
            (time, e.title.clone())
        })
        .collect();
    let next = upcoming
        .iter()
        .find(|e| !e.is_all_day() && e.instants(&tz).0 > now.timestamp());
    let tooltip = match (&store.auth, next) {
        (sync::AuthState::NeedsClient, _) => "Set up Google in Settings".to_string(),
        (sync::AuthState::SignedOut, _) => "Not signed in".to_string(),
        (_, Some(e)) => format!(
            "Next: {} at {}",
            e.title,
            ui::format::time(&e.start_zoned(&tz), clock)
        ),
        (_, None) => "Nothing else today".to_string(),
    };
    TrayContent {
        day: today.day() as u8,
        date_label: today.strftime("%A %-d %B").to_string(),
        agenda,
        tooltip,
    }
}
