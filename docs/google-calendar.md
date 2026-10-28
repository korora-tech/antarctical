# Google Calendar setup

[← Back to the README](../README.md)

## Request tester access

**Google Calendar integration through Antarctical's built-in client is currently available only to approved testers.** Email [david@rawkode.academy](mailto:david@rawkode.academy?subject=Antarctical%20tester%20access) with the Google account address you want to connect and ask to be added. Wait for access to be confirmed before signing in. Each Google account you connect needs to be on the tester list.

You can use [demo mode](../README.md#try-it-without-a-google-account) immediately, without joining the test or signing in.

While the Google OAuth project is in Testing, Google expires calendar authorizations and refresh tokens after seven days. Sign in again when prompted. See [Google's testing-mode documentation](https://support.google.com/cloud/answer/15549945?hl=en).

## Sign in

1. Click **Sign in with Google**. Your browser opens Google's account picker and consent page; approve it and return to Antarctical.
2. To add more Google accounts, click **Add another account** in Settings → Google accounts and pick the next account in the browser. Each account's calendars are listed under its email in the sidebar. **Remove** next to an account signs it out and revokes its token.

## Using your own OAuth client

Using your own client means you manage its access list separately from Antarctical's tester list. The built-in client's ID and secret are compiled into the binary. A Desktop client's secret cannot be kept confidential, and Google does not treat it as one; PKCE protects the sign-in. A build can substitute its own client by setting `ANTARCTICAL_GOOGLE_CLIENT_ID` and `ANTARCTICAL_GOOGLE_CLIENT_SECRET` at compile time. To use your own client at runtime instead:

1. In [Google Cloud](https://console.cloud.google.com/), create a project and [enable the Google Calendar API](https://console.cloud.google.com/apis/library/calendar-json.googleapis.com).
2. Configure the [Google Auth Platform](https://console.cloud.google.com/auth/overview) branding and audience. For an External app in Testing, add your Google account under **Audience → Test users**.
3. Under [Clients](https://console.cloud.google.com/auth/clients), create an OAuth client with application type **Desktop app**.
4. In Antarctical, open **Settings** (`ctrl-,`), paste the client ID and client secret, and click **Save to keyring**. Then sign in as above.

The OAuth client must be of type **Desktop app**. It has no redirect URI setting: Google accepts any `http://127.0.0.1:<port>` redirect for desktop clients. If Google reports `redirect_uri_mismatch`, check that you created a Desktop app client rather than a Web application client.

Sign-in uses the installed-app flow: authorization code with PKCE and a one-shot loopback redirect to `http://127.0.0.1:<random port>`. The scopes requested are `openid`, `userinfo.email`, `userinfo.profile`, `calendar` and `calendar.app.created`; the account's email is read from its primary calendar. Existing accounts with an older read-only grant retain it until they are added again.

## Where secrets live

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

## Further reference

- [Google OAuth for desktop apps](https://developers.google.com/identity/protocols/oauth2/native-app)
- [Privacy policy](https://antarctical.korora.tech/privacy-policy)
