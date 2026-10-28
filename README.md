# Antarctical

A native Google Calendar viewer for Linux, built with GPUI. Antarctical shows the calendars of one or more Google accounts in month, week, day and agenda views. Its penguin tray icon opens today's agenda, and it shows desktop notifications for event reminders. Antarctical only reads your calendars today, but it asks Google for full calendar access ahead of event editing.

Antarctical connects to Google with its own built-in OAuth client, or with one that **you** own if you prefer. Each account's refresh token, and your own client ID and secret if you save one, are stored only in your system keyring (the freedesktop Secret Service: GNOME Keyring, KWallet, KeePassXC…). Calendar data is kept in memory.

## Run

```sh
nix develop -c cargo run --release          # build and launch
nix run                                     # or build the package and launch it
nix develop -c cargo run -- --demo          # generated calendars, no Google account
nix develop -c cargo run -- --hidden        # start in the tray (for autostart)
```

The dev shell provides the Rust toolchain and GPUI's native dependencies. Wayland and X11 are both supported. The tray needs a StatusNotifierItem host (KDE, most wlroots/niri bars, or GNOME with the AppIndicator extension). Without one, Antarctical runs without a tray and quits when its window closes.

## Connect Google Calendar

1. Click **Sign in with Google**. Your browser opens Google's account picker and consent page; approve it and return to Antarctical.
2. To add more Google accounts, click **Add another account** in Settings → Google accounts and pick the next account in the browser. Each account's calendars are listed under its email in the sidebar. **Remove** next to an account signs it out and revokes its token.

### Using your own OAuth client

The built-in client's ID and secret are compiled into the binary. A Desktop client's secret cannot be kept confidential, and Google does not treat it as one; PKCE protects the sign-in. A build can substitute its own client by setting `ANTARCTICAL_GOOGLE_CLIENT_ID` and `ANTARCTICAL_GOOGLE_CLIENT_SECRET` at compile time. To use your own client at runtime instead:

1. In [Google Cloud](https://console.cloud.google.com/), create a project and [enable the Google Calendar API](https://console.cloud.google.com/apis/library/calendar-json.googleapis.com).
2. Configure the [OAuth consent screen](https://console.cloud.google.com/auth/overview). External is fine; add your Google account as a test user.
3. Under [Credentials](https://console.cloud.google.com/apis/credentials), create an **OAuth client ID** with application type **Desktop app**.
4. In Antarctical, open **Settings** (`ctrl-,`), paste the client ID and client secret, and click **Save to keyring**. Then sign in as above.

The OAuth client must be of type **Desktop app**. It has no redirect URI setting: Google accepts any `http://127.0.0.1:<port>` redirect for desktop clients. If Google reports `redirect_uri_mismatch`, the client was created as a Web application.

Sign-in uses the installed-app flow: authorization code with PKCE and a one-shot loopback redirect to `http://127.0.0.1:<random port>`. The scopes requested are `openid`, `userinfo.email`, `userinfo.profile`, `calendar` and `calendar.app.created`; the account's email is read from its primary calendar. Accounts signed in before this change keep their read-only grant until they are added again.

### Where secrets live

| Secret | Where |
|---|---|
| Your own OAuth client ID + secret (if saved) | Keyring item `Antarctical: Google OAuth client` (service `korora.tech.Antarctical`) |
| Refresh tokens, one per account | Keyring item `Antarctical: Google refresh tokens` (a JSON map of email → token) |
| Access token | Memory only |

- **Changing the client:** saving a different client ID (including your first one, replacing the built-in client) signs out every account, because a token belongs to the client that issued it. You then add your accounts again. A new secret for the same client ID keeps them.
- **Remove (an account):** revokes that account's token with Google and deletes it from the keyring.
- **Use built-in client:** deletes your saved client and every account, and goes back to the built-in client.
- **Expired sign-in:** if one account's sign-in expires or is revoked, only that account is dropped and named in the banner; the others keep syncing.
- **Client secret field:** never pre-filled. Leave it blank to keep the stored secret for the same client ID.

Preferences that are not secret (visible calendars, view, reminders, 24-hour clock, declined events) are saved in `$XDG_CONFIG_HOME/antarctical/settings.json`. `--demo` never writes this file.

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
- **Tray:** click the penguin icon for a compact calendar and agenda window. The icon stays the same as the date changes; today's date appears in its tooltip and menu. Choose a day there and open the full calendar at that date; use the tray menu to open the full calendar directly.
- **Navigation:** click a day in the month grid or in the sidebar's mini calendar to open it.
- **Calendar visibility:** checkboxes in the sidebar hide or show calendars. Until you toggle one, each calendar follows Google's own "show in list" setting.
- **Declined events:** hidden by default: in the views, the tray window, the tray menu's agenda and the Dux socket. Settings → Preferences → **Show events I declined** shows them greyed and struck through in the views and the tray window (the tray menu's agenda keeps skipping them). They never send reminders.
- **Single instance:** launching Antarctical while it runs shows the running one's window and exits (printing `already running (pid N); handing over`); a second `--hidden` launch just exits. Launches take turns on `$XDG_RUNTIME_DIR/antarctical.lock`, and the running instance is found through its Dux socket (below), so without `XDG_RUNTIME_DIR` two instances can still run. `--demo` runs alongside, without the socket.

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

## Dux shell integration

Antarctical feeds its events to the [Dux](https://github.com/duxlabs-dev) desktop shell (a DankMaterialShell fork) by serving the socket of Dux Calendar's `dankcal` daemon, read-only. Dux's bar clock, dashboard and calendar then show Antarctical's calendars.

- **Socket:** `$XDG_RUNTIME_DIR/dankcal-<pid>.sock` (mode 0600, created at start, removed on quit), or `$DANKCAL_SOCKET` when set. Set `DANKCAL_SOCKET` for both Dux and Antarctical, or for neither. Without a private `XDG_RUNTIME_DIR` there is no socket.
- **Dux setting:** set the calendar backend to `dankcal` (Settings → Time & Weather, or `"calendarBackend": "dankcal"` in Dux's `settings.json`). Dux only looks for the socket for about a minute after it starts or loses the connection, so start Antarctical with the session (`--hidden`), before Dux. On `auto` Dux looks only once, at start.
- **What Dux sees:** every calendar, read-only, with `hidden` following the sidebar checkboxes; events of the months Antarctical has loaded. When Dux asks for months that are not loaded, Antarctical answers with what it has, fetches them, and tells Dux to reload. Tasks are always empty; creating, editing or deleting from Dux is refused (`read-only`).
- **Declined events:** with **Show events I declined** on, they are served under a grey calendar `<calendar> (declined)` (id `<calendar id>#declined`), listed only while it has events. It is hidden when its calendar is.
- **Demo mode:** `--demo` never serves the socket, so Dux never sees generated events.
- **Real dankcal:** if a Dux Calendar daemon already runs, Antarctical warns that Dux may connect to either; if it owns `$DANKCAL_SOCKET`, Antarctical runs without a socket.

### Bar widget

`dux-plugin/` is a Dux bar widget (id `antarcticalNextEvent`) that shows the current or next timed event today in its calendar's colour: `now`, `in 12m`, or the start time. All-day and declined events are skipped, and the widget hides when nothing is left today. Clicking it opens Antarctical (by store path when built with Nix, otherwise from `PATH`); from five minutes before an event with a video call until it ends, a camera button joins the call (`xdg-open`). It reads Dux's own calendar data, so it needs the `dankcal` backend above.

```sh
nix build .#dux-plugin   # the plugin directory, for ~/.config/dms/plugins/antarcticalNextEvent
```

### Home Manager

```nix
{
  imports = [ antarctical.homeModules.default ];
  programs.antarctical = {
    enable = true;          # install the package
    service.enable = true;  # systemd user service: antarctical --hidden with the graphical session
    dux.enable = true;      # start before dms.service; with Dux's Home Manager module, also
                            # install the antarcticalNextEvent widget
    # dux.setCalendarBackend = true;  # see below
  };
}
```

- `dux.enable` needs `service.enable`. The widget is only installed: enable it in Dux's plugin settings and add it to a bar yourself. Installing it leaves Dux's `settings.json` and `plugin_settings.json` alone (Dux's module only writes `plugin_settings.json` when a plugin declares settings).
- `dux.setCalendarBackend` (off by default) sets Dux's `calendarBackend = "dankcal"` through `programs.dank-material-shell.settings`. That makes Home Manager own Dux's `~/.config/dms/settings.json` (a read-only link with only what Nix declares), so turn it on only if you already manage Dux's settings with Home Manager; otherwise choose the backend in Dux.
- The service restarts always, 10 s after it exits. A start while Antarctical already runs (opened by hand) logs one hand-off line and exits, and the service takes over once that instance quits. Quitting from the tray therefore lasts only until the next restart; `systemctl --user stop antarctical` closes it for the session.
- Antarctical's own `settings.json` is not managed: the app writes calendar visibility there.

## Development

```sh
nix develop -c cargo test                              # unit tests
nix develop -c cargo clippy --all-targets -- -D warnings
nix develop -c cargo fmt
nix flake check                                        # Home Manager module evaluation
```

| Module | Purpose |
|---|---|
| `src/google/oauth.rs` | PKCE, loopback redirect, token exchange/refresh/revoke |
| `src/google/api.rs` | `calendarList.list`, `events.list`, JSON → model |
| `src/secrets.rs` | Secret Service storage via `keyring-core` |
| `src/sync.rs` | Sync worker thread: credentials, fetching, retries |
| `src/demo.rs` | `--demo` worker with generated data |
| `src/store.rs` | UI state built from sync events |
| `src/tray.rs` | StatusNotifierItem tray and its fixed penguin icon |
| `src/branding.rs`, `data/branding/` | Embedded app artwork and shared website assets |
| `src/dankcal.rs` | dankcal socket for Dux; single-instance hand-off |
| `src/reminders.rs` | When reminders are due |
| `src/layout.rs` | Side-by-side placement of overlapping events |
| `src/ui/` | GPUI views: main window, calendar views, tray agenda, settings, text input |
| `dux-plugin/` | Dux bar widget (`packages.<system>.dux-plugin`) |
| `nix/hm-module.nix` | Home Manager module (`homeModules.default`) |

## Not yet

- Creating or editing events, and replying to invitations (v1 is read-only).
- An on-disk cache: data is fetched again at every start.

## License

Apache-2.0
