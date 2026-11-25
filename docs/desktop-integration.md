# Desktop integration

[← Back to the README](../README.md)

## Everyday use

- **Details:** click an event to see its time, place, guests and description. From there you can join its video call or open it in Google Calendar.
- **Tray:** click the penguin icon for a compact calendar and agenda window. The icon stays the same as the date changes; today's date appears in its tooltip and menu. Choose a day there and open the full calendar at that date; use the tray menu to open the full calendar directly.
- **Navigation:** click a day in the month grid or in the sidebar's mini calendar to open it.
- **Calendar visibility:** checkboxes in the sidebar hide or show calendars. Until you toggle one, each calendar follows Google's own "show in list" setting.
- **Declined events:** hidden by default: in the views, the tray window, the tray menu's agenda, the Dux socket, the vdir mirror and `antarctical events`. Settings → Preferences → **Show events I declined** shows them greyed and struck through in the views and the tray window (the tray menu's agenda keeps skipping them). They never send reminders.
- **Single instance:** launching Antarctical while it runs shows the running one's window and exits (printing `already running (pid N); handing over`); a second `--hidden` launch just exits. Launches take turns on `$XDG_RUNTIME_DIR/antarctical.lock`, and the running instance is found through its Dux socket (below), so without `XDG_RUNTIME_DIR` two instances can still run. `--demo` runs alongside, without the socket.

## Dux calendar backend

Antarctical feeds its events to the [Dux](https://github.com/duxlabs-dev) desktop shell (a DankMaterialShell fork) by serving the socket of Dux Calendar's `dankcal` daemon, read-only. Dux's bar clock, dashboard and calendar then show Antarctical's calendars.

- **Socket:** `$XDG_RUNTIME_DIR/dankcal-<pid>.sock` (mode 0600, created at start, removed on quit), or `$DANKCAL_SOCKET` when set. Set `DANKCAL_SOCKET` for both Dux and Antarctical, or for neither. Without a private `XDG_RUNTIME_DIR` there is no socket.
- **Dux setting:** set the calendar backend to `dankcal` (Settings → Time & Weather, or `"calendarBackend": "dankcal"` in Dux's `settings.json`). Dux only looks for the socket for about a minute after it starts or loses the connection, so start Antarctical with the session (`--hidden`), before Dux. On `auto` Dux looks only once, at start.
- **What Dux sees:** every calendar, read-only, with `hidden` following the sidebar checkboxes; events of the months Antarctical has loaded. When Dux asks for months that are not loaded, Antarctical answers with what it has, fetches them, and tells Dux to reload. Tasks are always empty; creating, editing or deleting from Dux is refused (`read-only`).
- **Declined events:** with **Show events I declined** on, they are served under a grey calendar `<calendar> (declined)` (id `<calendar id>#declined`), listed only while it has events. It is hidden when its calendar is.
- **Demo mode:** `--demo` never serves the socket, so Dux never sees generated events.
- **Real dankcal:** if a Dux Calendar daemon already runs, Antarctical warns that Dux may connect to either; if it owns `$DANKCAL_SOCKET`, Antarctical runs without a socket.

## Noctalia

Antarctical fits into the [Noctalia](https://noctalia.dev) shell in two parts, which work together or alone.

### Noctalia's own calendar

`antarctical vdir` mirrors your calendars into a directory that Noctalia reads, so the control center's Calendar tab, the desktop calendar widget and plugins using `noctalia.calendarEvents()` all show Antarctical's events. Antarctical stays the only thing signed in to Google.

```sh
antarctical vdir ~/.local/share/antarctical/calendars
```

```toml
# ~/.config/noctalia/config.toml
[calendar]
enabled = true

[calendar.account.antarctical]
type = "vdir"
name = "Antarctical"
path = "~/.local/share/antarctical/calendars"

# Antarctical already sends reminders; without this Noctalia sends its own as well.
[calendar.reminders]
enabled = false
```

- **What is mirrored:** the calendars ticked in Antarctical's sidebar, with their names and colours, one `.ics` file per event. Hidden calendars and declined events are left out. An event's link is its video call when it has one, otherwise its Google Calendar page.
- **Window:** last month to six months ahead; change it with `--back` and `--ahead` (up to ten months together). Noctalia shows nothing outside it.
- **Updates:** the command runs until stopped and rewrites only what changed, as Antarctical syncs. While Antarctical is not running the directory keeps the last mirror; it catches up when Antarctical is back.
- **On disk:** the directory and its files are readable only by you (0700/0600), like the offline copy, and hold titles, places, descriptions and links in plain text. Stop the command and delete the directory to remove them.
- **Shared directories:** only collections the command wrote (they carry a `.antarctical` file) are changed or removed, so the directory can also hold vdirsyncer or khal collections.

### Next-event bar widget

`noctalia-plugin/` is a Noctalia plugin (id `korora-tech/antarctical`). Its bar widget shows the current or next timed event today in its calendar's colour: `now`, `in 12m`, or the start time when that is more than an hour away. All-day and declined events are skipped. From five minutes before an event with a video call until it ends, a Join button opens the call. Hovering lists the rest of today; a click opens the control center's Calendar tab and a right click opens Antarctical. With nothing left today the widget is a single calendar glyph, or hidden if you prefer.

The plugin also adds a launcher provider: `/cal` followed by a few letters finds upcoming events by title, place or calendar. Enter joins the call when it is about to start, and otherwise opens that day in Antarctical.

Install it by linking the directory into Noctalia's local plugin directory, then enable it under **Settings → Plugins** and add the **Antarctical** widget to a bar:

```sh
ln -s "$PWD/noctalia-plugin" ~/.local/share/noctalia/plugins/antarctical
nix build github:korora-tech/antarctical#noctalia-plugin   # or the same directory, built with Nix
```

Declaratively, in Noctalia's config:

```toml
[plugins]
enabled = ["korora-tech/antarctical"]

[widget.antarctical]
type = "korora-tech/antarctical:bar"

[bar.default]
end = ["antarctical", "clock"]
```

The plugin needs `antarctical` on Noctalia's `PATH` (the Nix build uses its store path instead) and `xdg-open` for Join. Its settings cover how early Join is offered, how many days the launcher searches, and whether the title is shown.

## Command line

These talk to the running Antarctical over its socket and never start the app. They exit with status 3 when Antarctical is not running, except the two that run until stopped, which wait for it.

| Command | What it does |
| --- | --- |
| `antarctical events [--days N]` | Prints today and the following days (7 by default) as `{"events":[…]}`. |
| `antarctical events --watch` | Prints the agenda whenever it changes, one JSON object per line, for shell widgets. Runs until stopped. |
| `antarctical vdir DIR` | Mirrors the calendars into `DIR` (see above). Runs until stopped. |
| `antarctical show [--date YYYY-MM-DD]` | Brings the window forward, optionally on a date. |
| `antarctical refresh` | Syncs now. |

An event has `id`, `title`, `calendar`, `color`, `startMs`, `endMs` (milliseconds since the Unix epoch; all-day events run from local midnight to local midnight), `allDay`, `location`, `url`, `meetingUrl`, `tentative` and `declined`. Events of calendars hidden in the sidebar are left out, and the two links are only ever `http(s)` addresses.

`--watch` prints `{"type":"begin"}`, one `{"type":"event",…}` per event and `{"type":"end","events":N}` for each agenda, `{"type":"offline"}` while Antarctical is not running, and `{"type":"ping"}` every 30 seconds of silence.

## Home Manager

Add the repository to your existing flake's inputs:

```nix
inputs.antarctical.url = "github:korora-tech/antarctical";
```

Pass `antarctical` to your Home Manager modules with `extraSpecialArgs = { inherit antarctical; };` (or `home-manager.extraSpecialArgs` when using the NixOS module). Then add:

```nix
{ antarctical, ... }:
{
  imports = [ antarctical.homeModules.default ];
  programs.antarctical = {
    enable = true;          # install the package
    service.enable = true;  # systemd user service: antarctical --hidden with the graphical session
    noctalia.enable = true; # link the Noctalia plugin into ~/.local/share/noctalia/plugins
    vdir.enable = true;     # systemd user service: antarctical vdir, for Noctalia's calendar
    # vdir.path = "…";      # default: $XDG_DATA_HOME/antarctical/calendars
    # dux.enable = true;    # start before dms.service
    # dux.setCalendarBackend = true;  # see below
  };
}
```

- `noctalia.enable` only installs the plugin, with `antarctical`'s store path as its command: enable `korora-tech/antarctical` and place its widget in Noctalia's own configuration, as shown above. Remove a hand-made link at `~/.local/share/noctalia/plugins/antarctical` first, or Home Manager refuses to replace it.
- `vdir.enable` only writes the directory: add the `[calendar.account.…]` table shown above to Noctalia's configuration.
- `dux.enable` needs `service.enable`.
- `dux.setCalendarBackend` (off by default) sets Dux's `calendarBackend = "dankcal"` through `programs.dank-material-shell.settings`. That makes Home Manager own Dux's `~/.config/dms/settings.json` (a read-only link with only what Nix declares), so turn it on only if you already manage Dux's settings with Home Manager; otherwise choose the backend in Dux.
- The service restarts always, 10 s after it exits. A start while Antarctical already runs (opened by hand) logs one hand-off line and exits, and the service takes over once that instance quits. Quitting from the tray therefore lasts only until the next restart; `systemctl --user stop antarctical` closes it for the session.
- Antarctical's own `settings.json` is not managed: the app writes calendar visibility there.
