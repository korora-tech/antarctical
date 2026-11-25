# Changelog

All notable changes to Antarctical are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- The per-account opt-in is now **Edit events and reply to invitations**. It uses the same `calendar.events` scope as before, so existing opt-ins keep working.
- Privacy policy lists the OAuth scopes the app actually requests (`calendar.readonly` by default; `calendar.events` and `calendar.calendarlist.readonly` only after enabling Edit events and reply to invitations).

### Added

- **Click-drag to create:** in the week and day views, drag across empty space in a day to pick a time range (15-minute steps, up or down). On release the quick-add bar opens with that day and time already filled in; type the title and press Enter. Text that names its own time still wins.
- **Drag to move and resize:** in the week and day views, drag a timed event to another time or day, or drag its top or bottom edge to change its start or end. Steps are 15 minutes, the block follows the pointer, and the change is sent to Google when you let go (and rolls back with a message if refused). Only events on accounts with editing enabled, that you organise or that have no guests, and that sit within one day can be dragged; dragging a repeating event changes that occurrence only.
- **Offline copy:** the last-synced calendars and events are saved to `$XDG_CACHE_HOME/antarctical/cache.json` (mode 0600) and shown immediately at startup while Google is contacted. Settings → Preferences turns it off and deletes the file; so does signing out of your last account. Not Google sync tokens (they cannot be combined with the time-windowed fetch), and offline edits are not queued.
- **Quick add:** `n` (or Ctrl+N, or the New event button) opens a one-line bar that understands `Lunch with Sam tomorrow 1pm at Dishoom`: dates, weekdays, times and ranges, durations, locations, guest emails, `/calendar`, `+meet`, and repeats such as `every weekday`. A live preview shows what will be created; Enter adds it.
- **Create, edit and delete events:** changes appear at once and roll back with a message if Google refuses them. The editor (`e` on a selected event) changes title, time and location. Editing or deleting one occurrence of a repeating event affects only that occurrence.
- Release workflow: pushing a `vX.Y.Z` tag publishes a GitHub release with a binary tarball and checksums.
- `docs/oauth-verification.md`: scope rationale and Google OAuth verification checklist.
