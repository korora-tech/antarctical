# Changelog

All notable changes to Antarctical are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- Privacy policy lists the OAuth scopes the app actually requests (`calendar.readonly` by default; `calendar.events` and `calendar.calendarlist.readonly` only after enabling Reply to invitations).

### Added

- Release workflow: pushing a `vX.Y.Z` tag publishes a GitHub release with a binary tarball and checksums.
- `docs/oauth-verification.md`: scope rationale and Google OAuth verification checklist.
