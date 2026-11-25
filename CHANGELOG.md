# Changelog

All notable changes to Antarctical are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- The per-account opt-in is now **Edit events and reply to invitations**. It uses the same `calendar.events` scope as before, so existing opt-ins keep working.
- Privacy policy lists the OAuth scopes the app actually requests (`calendar.readonly` by default; `calendar.events` and `calendar.calendarlist.readonly` only after enabling Edit events and reply to invitations).

### Added

- **Quick add:** `n` (or Ctrl+N, or the New event button) opens a one-line bar that understands `Lunch with Sam tomorrow 1pm at Dishoom`: dates, weekdays, times and ranges, durations, locations, guest emails, `/calendar`, `+meet`, and repeats such as `every weekday`. A live preview shows what will be created; Enter adds it.
- **Create, edit and delete events:** changes appear at once and roll back with a message if Google refuses them. The editor (`e` on a selected event) changes title, time and location. Editing or deleting one occurrence of a repeating event affects only that occurrence.
- Release workflow: pushing a `vX.Y.Z` tag publishes a GitHub release with a binary tarball and checksums.
- `docs/oauth-verification.md`: scope rationale and Google OAuth verification checklist.
