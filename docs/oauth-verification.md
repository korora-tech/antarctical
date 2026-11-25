# Google OAuth verification

Antarctical's OAuth app is in **Testing**: only hand-approved test users can sign in, and their refresh tokens expire after 7 days. This page is the plan and the material for leaving Testing. The consent screen, the code and the documentation must all describe the same scopes; reviewers compare them.

## Scopes

Calendar scopes are **sensitive**, not restricted, so no third-party security assessment (CASA) is required. Confirm the classification on the Data Access page in the Cloud Console before submitting.

| Scope | When requested | Used for |
|---|---|---|
| `calendar.readonly` | Every sign-in (default) | `calendarList.list`, `events.list`, `colors.get`: show calendars and events, drive reminders |
| `calendar.events` | Only after the user enables **Edit events and reply to invitations** for an account (replaces `calendar.readonly` for that account) | `events.patch` to set the user's own attendee response; `events.insert`, `events.patch` and `events.delete` for events the user creates, edits or deletes in the app |
| `calendar.calendarlist.readonly` | With `calendar.events` | `calendarList.list` and the account email (primary calendar id). `calendar.events` cannot list calendars |

Nothing else is requested: no `openid`, `userinfo.*`, full `calendar`, or `calendar.app.created`. Those came from commit `eecce94` and were removed by the opt-in RSVP work; the consent screen in the Cloud Console may still list them and must be trimmed to match.

Why not narrower:

- `calendar.events.owned` cannot answer invitations, because invitations live on calendars the user does not own.
- `calendar.events.readonly` plus `calendar.calendarlist.readonly` would replace `calendar.readonly` (two scopes instead of one, same data), so there is no gain.
- Future features need no new scope except Google Tasks (`tasks`, requested later by incremental consent). Do not add it to the consent screen until the feature ships, or reviewers will ask for it in the demo.

Phase 1 editing fits inside the set above. Declare all three scopes in one verification so editing does not trigger a second review.

## What only David can do (Google Cloud Console)

1. **Domain:** verify `korora.tech` in Google Search Console with an owner account of the Cloud project, then add it under Branding → Authorized domains.
2. **Branding:** app name `Antarctical`, 120x120 logo (`data/branding/antarctical-app-icon.png` resized), support email, developer contact email, homepage `https://antarctical.korora.tech/`, privacy policy `https://antarctical.korora.tech/privacy-policy`, terms `https://antarctical.korora.tech/terms-of-service`. All three pages must be live on the verified domain and the homepage must link to the privacy policy (it does).
3. **Data Access:** remove every scope except the three above, paste the justifications below.
4. **Deploy the website** from this branch first, so the live privacy policy matches the consent screen.
5. **Record and upload the demo video** (unlisted YouTube), script below.
6. **Publish app:** Audience → **Publish app** (In production), then **Prepare for verification**, then submit. Publishing ends the 7-day token expiry and the test-user list immediately; until verification passes, new users see an "unverified app" warning and the project is capped at 100 users. That makes Publish worth doing now, before the review finishes.
7. **Answer reviewer emails** at the developer contact address. Slow replies are the usual cause of long reviews.

The built-in client ID in `src/google/oauth.rs` must belong to the project being verified.

## Scope justifications (paste into the form)

**`calendar.readonly`:**
Antarctical is a desktop calendar app. It uses this scope to list the signed-in user's calendars and fetch their events so it can display month, week, day and agenda views and show desktop reminders. Data is fetched directly from Google to the user's computer and is not sent to any server operated by us. The scope is read-only; a narrower scope is not available because the app needs events and the calendar list from every calendar the user can see, including calendars shared with them.

**`calendar.events`:**
Used only when the user turns on "Edit events and reply to invitations" for an account. The app then sends the user's accept, decline or maybe response to an invitation by updating the user's own attendee status on that event. Invitations are on calendars the user does not own, so `calendar.events.owned` cannot be used. The app does not request this scope unless the user enables the feature, and replaces the read-only grant for that account. The same scope lets the user create, edit and delete events from inside Antarctical: a quick-add bar and an editor call `events.insert`, `events.patch` and `events.delete` only when the user submits them. When the user types a guest's email into a new event, Google sends that guest an invitation.

**`calendar.calendarlist.readonly`:**
Requested together with `calendar.events`, which does not allow listing calendars. The app needs the calendar list to show the user's calendars, and the account's email address (the ID of the primary calendar) to tell accounts apart.

## Demo video script (3 to 4 minutes, English, one take)

Show the browser address bar with the OAuth client ID during consent. Record the desktop at 1080p.

1. **Intro (15 s):** "This is Antarctical, a desktop calendar for Linux. It talks to Google Calendar directly from this computer."
2. **Sign-in, read-only (45 s):** Settings → Add account. Show the browser opening, the full URL (client ID visible, `scope=...calendar.readonly`), the consent screen listing the single read-only permission, then approve. Return to the app.
3. **Use of `calendar.readonly` (45 s):** show the calendar list with show/hide toggles, month, week and agenda views, event details, and a desktop reminder notification.
4. **Opt in to replies (60 s):** Settings → enable **Edit events and reply to invitations** for the account. Show the new consent URL with `calendar.events` and `calendar.calendarlist.readonly`, the consent screen, then approve.
5. **Use of `calendar.events` (45 s):** open an invitation, click Accept, show the status change in Antarctical and then in the Google Calendar web UI.
6. **Data handling and revocation (30 s):** show the keyring entry holding only the refresh token, remove the account in Settings, then show it gone from https://myaccount.google.com/permissions.
7. **Privacy (10 s):** show the privacy policy page at the verified domain.

Before recording, use a Google account that is not a developer on the project, so the video shows the real end-user consent screen.

## Keep in sync

- **Privacy policy:** says calendar data is held in memory and in an optional local offline copy (`cache.json`). Keep that sentence, the home page privacy note, the README and `docs/google-calendar.md` aligned with `src/cache.rs`.
- **Editing:** when create/edit/delete ships, update the `calendar.events` justification and re-record step 5 if Google asks. Do not add scopes without updating the consent screen first; a scope missing from it makes sign-in fail with `invalid_scope`.
