# Antarctical

A native Google Calendar viewer for Linux, built with GPUI. Antarctical shows the calendars of one or more Google accounts in month, week, day and agenda views. It puts today's date and agenda in the system tray and shows desktop notifications for event reminders. Access is read-only: Antarctical never changes your calendar.

Antarctical connects to Google with an OAuth client that **you** own. The client ID, client secret and each account's refresh token are stored only in your system keyring (the freedesktop Secret Service: GNOME Keyring, KWallet, KeePassXC…). Calendar data is kept in memory.

## Run

```sh
nix develop -c cargo run --release          # build and launch
nix run                                     # or build the package and launch it
nix develop -c cargo run -- --demo          # generated calendars, no Google account
nix develop -c cargo run -- --hidden        # start in the tray (for autostart)
```

The dev shell provides the Rust toolchain and GPUI's native dependencies. Wayland and X11 are both supported. The tray needs a StatusNotifierItem host (KDE, most wlroots/niri bars, or GNOME with the AppIndicator extension). Without one, Antarctical runs without a tray and quits when its window closes.

## Connect Google Calendar

1. In [Google Cloud](https://console.cloud.google.com/), create a project and [enable the Google Calendar API](https://console.cloud.google.com/apis/library/calendar-json.googleapis.com).
2. Configure the [OAuth consent screen](https://console.cloud.google.com/auth/overview). External is fine; add your Google account as a test user.
3. Under [Credentials](https://console.cloud.google.com/apis/credentials), create an **OAuth client ID** with application type **Desktop app**.
4. In Antarctical, open **Settings** (`ctrl-,`), paste the client ID and client secret, and click **Save to keyring**.
5. Click **Sign in with Google**. Your browser opens Google's account picker and consent page; approve it and return to Antarctical.
6. To add more Google accounts, click **Add another account** in Settings → Google accounts and pick the next account in the browser. Each account's calendars are listed under its email in the sidebar. **Remove** next to an account signs it out and revokes its token.

The OAuth client must be of type **Desktop app**. It has no redirect URI setting: Google accepts any `http://127.0.0.1:<port>` redirect for desktop clients. If Google reports `redirect_uri_mismatch`, the client was created as a Web application.

Sign-in uses the installed-app flow: authorization code with PKCE and a one-shot loopback redirect to `http://127.0.0.1:<random port>`. The only scope requested is `calendar.readonly`; the account's email is read from its primary calendar.

### Where secrets live

| Secret | Where |
|---|---|
| OAuth client ID + secret | Keyring item `Antarctical: Google OAuth client` (service `com.korora.Antarctical`) |
| Refresh tokens, one per account | Keyring item `Antarctical: Google refresh tokens` (a JSON map of email → token) |
| Access token | Memory only |

- **Changing the client:** saving a different client ID signs out every account, because a token belongs to the client that issued it. You then add your accounts again. A new secret for the same client ID keeps them.
- **Remove (an account):** revokes that account's token with Google and deletes it from the keyring.
- **Remove from keyring (the client):** deletes the client and every account.
- **Expired sign-in:** if one account's sign-in expires or is revoked, only that account is dropped and named in the banner; the others keep syncing.
- **Client secret field:** never pre-filled. Leave it blank to keep the stored secret for the same client ID.

Preferences that are not secret (visible calendars, view, reminders, 24-hour clock) are saved in `$XDG_CONFIG_HOME/antarctical/settings.json`. `--demo` never writes this file.

## Using it

| Key | Action |
|---|---|
| `t` | Today |
| `←` `→` / `h` `l` | Previous / next month, week, day or agenda page |
| `m` `w` `d` `a` | Month, week, day, agenda |
| `esc` | Close the event panel, or leave Settings |
| `ctrl-,` | Settings |
| `ctrl-r` | Refresh now |
| `ctrl-w` | Close the window (Antarctical stays in the tray) |
| `ctrl-q` | Quit |

- **Details:** click an event to see its time, place, guests and description. From there you can join its video call or open it in Google Calendar.
- **Navigation:** click a day in the month grid or in the sidebar's mini calendar to open it.
- **Calendar visibility:** checkboxes in the sidebar hide or show calendars. Until you toggle one, each calendar follows Google's own "show in list" setting.
- **Declined events:** shown struck through.

## Sync

- **Recurring events:** Google expands them (`singleEvents=true`), so every instance is a plain event.
- **What gets fetched:** each view asks for the months it shows. The previous, current and next month are always kept fresh for the tray and reminders.
- **Refresh:** every 5 minutes, and on `ctrl-r`.
- **Keyring locked at start:** Antarctical says so and waits; unlock it and press **Retry** (or `ctrl-r`) to read the credentials again.
- **Banner:** sync problems show at the top. Messages about dropped accounts stay until you add the account again or close the banner with ✕.
- **Errors:** after a network error the banner says so, the current data stays on screen, and the sync is retried a minute later. A calendar that Google refuses (unsubscribed, permissions changed) is skipped and named in the banner.

## Reminders

Popup reminders from Google become desktop notifications (org.freedesktop.Notifications).
- **Defaults:** a calendar's default reminders apply to timed events that use defaults.
- **Skipped:** declined events, and events on calendars hidden in the sidebar, never notify. A reminder that came due while Antarctical was not running is still shown for up to 10 minutes.
- **Off switch:** Settings → Preferences.

## Development

```sh
nix develop -c cargo test                              # unit tests
nix develop -c cargo clippy --all-targets -- -D warnings
nix develop -c cargo fmt
```

| Module | Purpose |
|---|---|
| `src/google/oauth.rs` | PKCE, loopback redirect, token exchange/refresh/revoke |
| `src/google/api.rs` | `calendarList.list`, `events.list`, JSON → model |
| `src/secrets.rs` | Secret Service storage via `keyring-core` |
| `src/sync.rs` | Sync worker thread: credentials, fetching, retries |
| `src/demo.rs` | `--demo` worker with generated data |
| `src/store.rs` | UI state built from sync events |
| `src/tray.rs` | StatusNotifierItem tray and its date icon |
| `src/reminders.rs` | When reminders are due |
| `src/layout.rs` | Side-by-side placement of overlapping events |
| `src/ui/` | GPUI views: main window, calendar views, settings, text input |

## Not yet

- Creating or editing events, and replying to invitations (v1 is read-only).
- An on-disk cache: data is fetched again at every start.
- Single-instance handling: launching it twice gives two windows and two tray icons.

## License

Apache-2.0
