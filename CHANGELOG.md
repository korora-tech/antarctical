# Changelog

All notable changes to Antarctical are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- The per-account opt-in is now **Edit events and reply to invitations**. It uses the same `calendar.events` scope as before, so existing opt-ins keep working.
- Privacy policy lists the OAuth scopes the app actually requests (`calendar.readonly` by default; `calendar.events` and `calendar.calendarlist.readonly` only after enabling Edit events and reply to invitations).

### Removed

- The Dux next-event widget (`dux-plugin/`, the `dux-plugin` package, and its installation by `programs.antarctical.dux.enable`). The Dux calendar backend is unchanged.

### Added

- **Calendar sets:** save the calendars you are showing as a named set in Settings → Preferences, then switch between sets with one click on the chips above the sidebar's calendar list (the chip matching what is shown is highlighted). Sets are stored in settings.json, survive removing an account, and saving with an existing name replaces it.
- **Extra time zones:** Settings → Preferences → Extra time zones takes up to two IANA names (for example `Europe/London, Asia/Tokyo`) and shows what each clock reads beside the hour gutter of the week and day views, headed by the zone's abbreviation. Names are checked when applied. Events still display in your system time zone; per-event zones are not supported yet.
- **Noctalia:** `antarctical vdir DIR` mirrors the calendars shown in the sidebar into a vdir that Noctalia's own calendar reads (a `type = "vdir"` account), so its Calendar tab shows your events. `noctalia-plugin/` adds a bar widget with the current or next event today and a Join button for video calls, plus `/cal` launcher search. Home Manager: `programs.antarctical.noctalia.enable` and `programs.antarctical.vdir.enable`.
- **Command line:** `antarctical events [--days N] [--watch]` prints the running app's agenda as JSON, `antarctical show [--date YYYY-MM-DD]` brings its window forward on a date, and `antarctical refresh` syncs now.
- **Notification actions:** reminder notifications now have a Join button (events with a video call) that opens the call, and Snooze 5 min, which shows the reminder again with the time left worked out afresh (dropped if the event is over by then). Snoozes are kept in memory only, so quitting the app cancels them. Notifications stay up for 30 seconds instead of 15.
- **Edit or delete a whole repeating series:** on one occurrence of a series you organise, the editor has a "Changes apply to" button. Choose "All events in the series" to change the title, location, guests and reminders of every occurrence, or to move them all to a new time of day and length (their dates stay), or to delete the series. Occurrences you changed individually keep their own edits where Google does. Not offered for "this and following".
- **Move an event to another calendar:** the editor has a Calendar button (shown for events you organise, not single occurrences of a series) that steps through the other writable calendars of the same account. Saving moves the event with Google's move call and applies any other edits afterwards; if the move fails the event goes back, and if only the later edit fails you are told the event was moved.
- **Read-only calendars are recognised:** the calendar list now carries your access role. Holiday calendars and shared calendars you can only view no longer offer Edit event, cannot be dragged on, are skipped when quick add picks a calendar, and say "is read-only for you" if you name one with `/name`. The offline copy from before this change is discarded once and rebuilt on the next sync.
- **Edit guests and reminders:** the event editor has Guests (emails, separated by commas; only for events you organise) and Reminders (`10m, 1h, 1d`, `default` or `none`; timed events). Guest changes are merged into the guest list Google holds at save time, keep everyone's replies, never remove you, the organiser or rooms, are refused if the event changed in the meantime, and email new guests an invitation. Setting reminders replaces all of an event's reminders with pop-ups.
- **Click-drag to create:** in the week and day views, drag across empty space in a day to pick a time range (15-minute steps, up or down). On release the quick-add bar opens with that day and time already filled in; type the title and press Enter. Text that names its own time still wins.
- **Drag to move and resize:** in the week and day views, drag a timed event to another time or day, or drag its top or bottom edge to change its start or end. Steps are 15 minutes, the block follows the pointer, and the change is sent to Google when you let go (and rolls back with a message if refused). Only events on accounts with editing enabled, that you organise or that have no guests, and that sit within one day can be dragged; dragging a repeating event changes that occurrence only.
- **Offline copy:** the last-synced calendars and events are saved to `$XDG_CACHE_HOME/antarctical/cache.json` (mode 0600) and shown immediately at startup while Google is contacted. Settings → Preferences turns it off and deletes the file; so does signing out of your last account. Not Google sync tokens (they cannot be combined with the time-windowed fetch), and offline edits are not queued.
- **Quick add:** `n` (or Ctrl+N, or the New event button) opens a one-line bar that understands `Lunch with Sam tomorrow 1pm at Dishoom`: dates, weekdays, times and ranges, durations, locations, guest emails, `/calendar`, `+meet`, and repeats such as `every weekday`. A live preview shows what will be created; Enter adds it.
- **Create, edit and delete events:** changes appear at once and roll back with a message if Google refuses them. The editor (`e` on a selected event) changes title, time and location. Editing or deleting one occurrence of a repeating event affects only that occurrence.
- Release workflow: pushing a `vX.Y.Z` tag publishes a GitHub release with a binary tarball and checksums.
- `docs/oauth-verification.md`: scope rationale and Google OAuth verification checklist.
