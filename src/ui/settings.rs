//! Settings: the Google OAuth client (stored in the keyring), the account, and preferences.
//!
//! The client secret is write-only from the UI's point of view: it goes to the sync worker,
//! which stores it in the keyring; the field is cleared afterwards and never pre-filled.

use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    ScrollHandle, SharedString, Subscription, Window, div, prelude::*, px, rgb,
};

use super::text_input::{Submit, TextInput};
use super::theme::*;
use super::widgets::{ButtonStyle, OnClick, body_text, button, caption, card, checkbox_row};
use crate::store::Store;
use crate::sync::{AuthState, Command};

const CREDENTIALS_URL: &str = "https://console.cloud.google.com/apis/credentials";
const CALENDAR_API_URL: &str =
    "https://console.cloud.google.com/apis/library/calendar-json.googleapis.com";
const CONSENT_URL: &str = "https://console.cloud.google.com/auth/overview";

pub enum SettingsEvent {
    Close,
}

pub struct SettingsView {
    store: Entity<Store>,
    client_id: Entity<TextInput>,
    client_secret: Entity<TextInput>,
    /// The stored client id last copied into the field, so edits are not overwritten.
    shown_client: Option<Option<String>>,
    focus: FocusHandle,
    scroll: ScrollHandle,
    _subscription: Subscription,
}

impl EventEmitter<SettingsEvent> for SettingsView {}

impl SettingsView {
    pub fn new(store: Entity<Store>, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let client_id =
            cx.new(|cx| TextInput::new("1234567890-abc.apps.googleusercontent.com", false, 1, cx));
        let client_secret = cx.new(|cx| TextInput::new("GOCSPX-…", true, 2, cx));
        let subscription = cx.observe(&store, |this, _, cx| {
            this.sync_from_store(cx);
            cx.notify();
        });
        let mut this = Self {
            store,
            client_id,
            client_secret,
            shown_client: None,
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _subscription: subscription,
        };
        this.sync_from_store(cx);
        this
    }

    pub fn first_focus(&self, cx: &App) -> FocusHandle {
        self.client_id.focus_handle(cx)
    }

    /// When the stored client changes (loaded, saved, removed), reflect it in the fields.
    fn sync_from_store(&mut self, cx: &mut Context<Self>) {
        let stored = self.store.read(cx).client_id.clone();
        if self.shown_client.as_ref() == Some(&stored) {
            return;
        }
        self.shown_client = Some(stored.clone());
        let has_client = stored.is_some();
        self.client_id
            .update(cx, |i, cx| i.set_text(stored.unwrap_or_default(), cx));
        self.client_secret.update(cx, |i, cx| {
            i.set_text("", cx);
            i.set_placeholder(
                if has_client {
                    "Stored in keyring · leave blank to keep it"
                } else {
                    "GOCSPX-…"
                },
                cx,
            );
        });
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        let client_id = self.client_id.read(cx).text().trim().to_string();
        let secret = self.client_secret.read(cx).text().trim().to_string();
        self.store.read(cx).send(Command::SaveClient {
            client_id,
            client_secret: (!secret.is_empty()).then_some(secret),
        });
        // Do not keep the secret around in the UI once it has been handed over.
        self.client_secret.update(cx, |i, cx| {
            i.set_text("", cx);
            i.set_masked(true, cx);
        });
    }

    fn click(
        &self,
        cx: &Context<Self>,
        f: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> OnClick {
        Box::new(cx.listener(move |this, _: &ClickEvent, window, cx| f(this, window, cx)))
    }

    fn send(&self, cx: &Context<Self>, cmd: fn() -> Command) -> OnClick {
        Box::new(cx.listener(move |this, _: &ClickEvent, _, cx| this.store.read(cx).send(cmd())))
    }

    fn field(label: &'static str, hint: &'static str, input: Entity<TextInput>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap(px(6.0))
            .child(caption(label))
            .child(input)
            .child(div().text_size(px(11.0)).text_color(rgb(MUTED)).child(hint))
    }

    fn step(n: &'static str, text: &'static str) -> gpui::Div {
        div()
            .flex()
            .gap_3()
            .child(
                div()
                    .size(px(20.0))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(rgb(SELECTED))
                    .text_size(px(11.0))
                    .font_weight(FontWeight::BOLD)
                    .text_color(rgb(ACCENT_TEXT))
                    .child(n),
            )
            .child(body_text(text))
    }
}

impl Focusable for SettingsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let auth = store.auth.clone();
        let accounts = store.accounts.clone();
        let signing_in = store.signing_in.clone();
        let demo = store.demo;
        let stored_id = store.client_id.clone();
        let reminders = store.prefs.reminders;
        let clock_24h = store.prefs.clock_24h;
        let secret_masked = self.client_secret.read(cx).is_masked();

        let client_card = card("Google OAuth client")
            .child(body_text(
                "Antarctical connects to Google with an OAuth client that you own. Create one once; \
                 the client ID and secret are stored only in your system keyring \
                 (Secret Service: GNOME Keyring, KWallet, KeePassXC…).",
            ))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(Self::step("1", "In Google Cloud, create a project and enable the Google Calendar API."))
                    .child(Self::step("2", "Configure the OAuth consent screen (External is fine) and add your Google account as a test user."))
                    .child(Self::step("3", "Create credentials → OAuth client ID → Application type: Desktop app."))
                    .child(Self::step("4", "Paste the client ID and client secret below and save.")),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_2()
                    .child(button("open-api", "Enable Calendar API", ButtonStyle::Secondary, Some(Box::new(|_, _, cx| cx.open_url(CALENDAR_API_URL)))))
                    .child(button("open-consent", "Consent screen", ButtonStyle::Secondary, Some(Box::new(|_, _, cx| cx.open_url(CONSENT_URL)))))
                    .child(button("open-creds", "Credentials", ButtonStyle::Secondary, Some(Box::new(|_, _, cx| cx.open_url(CREDENTIALS_URL))))),
            )
            .child(Self::field(
                "CLIENT ID",
                "Ends in .apps.googleusercontent.com",
                self.client_id.clone(),
            ))
            .child(
                div()
                    .flex()
                    .items_end()
                    .gap_2()
                    .child(div().flex_1().child(Self::field(
                        "CLIENT SECRET",
                        "Never shown again after saving",
                        self.client_secret.clone(),
                    )))
                    .child(div().pb(px(22.0)).child(button(
                        "toggle-secret",
                        if secret_masked { "Show" } else { "Hide" },
                        ButtonStyle::Secondary,
                        Some(self.click(cx, |this, _, cx| {
                            this.client_secret.update(cx, |i, cx| {
                                let masked = i.is_masked();
                                i.set_masked(!masked, cx)
                            })
                        })),
                    ))),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().size(px(7.0)).rounded_full().bg(rgb(if stored_id.is_some() { GREEN } else { DISABLED })))
                            .child(
                                div()
                                    .text_size(px(11.0))
                                    .text_color(rgb(MUTED))
                                    .child(match (&stored_id, demo) {
                                        (_, true) => "Demo mode: kept in memory, not in the keyring".to_string(),
                                        (Some(_), false) => "Client ID and secret are stored in the keyring".to_string(),
                                        (None, false) => "Nothing stored yet".to_string(),
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_2()
                            .when(stored_id.is_some(), |d| {
                                d.child(button(
                                    "remove-client",
                                    "Remove from keyring",
                                    ButtonStyle::Danger,
                                    Some(self.send(cx, || Command::ClearClient)),
                                ))
                            })
                            .child(button(
                                "save-client",
                                "Save to keyring",
                                ButtonStyle::Primary,
                                Some(self.click(cx, |this, _, cx| this.save(cx))),
                            )),
                    ),
            );

        let account_card = {
            let intro = match &auth {
                AuthState::Starting => "Reading the keyring…",
                AuthState::NeedsClient => "Save an OAuth client above first.",
                AuthState::Demo => "Demo mode: showing generated accounts.",
                AuthState::Ready if accounts.is_empty() => {
                    "No account yet. Each account gets read-only access; its refresh token is kept in the keyring."
                }
                AuthState::Ready => {
                    "Read-only access. Each account's refresh token is kept in the keyring; removing an account revokes it."
                }
            };
            let can_add = auth == AuthState::Ready;
            let rows = accounts.iter().map(|email| {
                let target = email.clone();
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(BORDER))
                    .bg(rgb(PANEL_RAISED))
                    .px_3()
                    .py_2()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .min_w(px(0.0))
                            .child(
                                div()
                                    .size(px(7.0))
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .bg(rgb(GREEN)),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(12.0))
                                    .text_color(rgb(TEXT))
                                    .child(email.clone()),
                            ),
                    )
                    .when(can_add, |d| {
                        d.child(button(
                            SharedString::from(format!("remove-{email}")),
                            "Remove",
                            ButtonStyle::Danger,
                            Some(self.click(cx, move |this, _, cx| {
                                this.store
                                    .read(cx)
                                    .send(Command::RemoveAccount(target.clone()))
                            })),
                        ))
                    })
            });
            let footer = match (&signing_in, can_add) {
                (Some(url), _) => {
                    let url = url.clone();
                    div()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .child(body_text(
                            "Waiting for you to choose an account and approve access in the browser…",
                        ))
                        .child(
                            div()
                                .flex()
                                .gap_2()
                                .child(button(
                                    "copy-sign-in",
                                    "Copy sign-in link",
                                    ButtonStyle::Secondary,
                                    Some(self.click(cx, move |_, _, cx| {
                                        cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                            url.clone(),
                                        ))
                                    })),
                                ))
                                .child(button(
                                    "cancel-sign-in",
                                    "Cancel",
                                    ButtonStyle::Secondary,
                                    Some(self.send(cx, || Command::CancelSignIn)),
                                )),
                        )
                }
                (None, true) => div().flex().child(button(
                    "add-account",
                    if accounts.is_empty() {
                        "Sign in with Google"
                    } else {
                        "Add another account"
                    },
                    ButtonStyle::Primary,
                    Some(self.send(cx, || Command::AddAccount)),
                )),
                (None, false) => div(),
            };
            card("Google accounts")
                .child(body_text(intro))
                .child(div().flex().flex_col().gap_2().children(rows))
                .child(footer)
        };

        let prefs_card = card("Preferences")
            .child(checkbox_row(
                "pref-reminders",
                "Desktop notifications for event reminders",
                ACCENT,
                reminders,
                self.click(cx, |this, _, cx| {
                    this.store.update(cx, |s, cx| {
                        s.prefs.reminders = !s.prefs.reminders;
                        s.prefs.save();
                        cx.notify();
                    })
                }),
            ))
            .child(checkbox_row(
                "pref-clock",
                "24-hour clock",
                ACCENT,
                clock_24h,
                self.click(cx, |this, _, cx| {
                    this.store.update(cx, |s, cx| {
                        s.prefs.clock_24h = !s.prefs.clock_24h;
                        s.prefs.save();
                        cx.notify();
                    })
                }),
            ));

        div()
            .id("settings")
            .key_context("Settings")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &Submit, _, cx| this.save(cx)))
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                div()
                    .w_full()
                    .max_w(px(720.0))
                    .flex()
                    .flex_col()
                    .gap_4()
                    .p_6()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_size(px(18.0))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child("Settings"),
                            )
                            .child(button(
                                "settings-done",
                                "Done",
                                ButtonStyle::Secondary,
                                Some(self.click(cx, |_, _, cx| cx.emit(SettingsEvent::Close))),
                            )),
                    )
                    .child(client_card)
                    .child(account_card)
                    .child(prefs_card),
            )
    }
}
