# Desktop integration

[← Back to the README](../README.md)

## Everyday use

- **Details:** click an event to see its time, place, guests and description. From there you can join its video call or open it in Google Calendar.
- **Tray:** click the penguin icon for a compact calendar and agenda window. The icon stays the same as the date changes; today's date appears in its tooltip and menu. Choose a day there and open the full calendar at that date; use the tray menu to open the full calendar directly.
- **Navigation:** click a day in the month grid or in the sidebar's mini calendar to open it.
- **Calendar visibility:** checkboxes in the sidebar hide or show calendars. Until you toggle one, each calendar follows Google's own "show in list" setting.
- **Declined events:** hidden by default: in the views, the tray window, the tray menu's agenda and the Dux socket. Settings → Preferences → **Show events I declined** shows them greyed and struck through in the views and the tray window (the tray menu's agenda keeps skipping them). They never send reminders.
- **Single instance:** launching Antarctical while it runs shows the running one's window and exits (printing `already running (pid N); handing over`); a second `--hidden` launch just exits. Launches take turns on `$XDG_RUNTIME_DIR/antarctical.lock`, and the running instance is found through its Dux socket (below), so without `XDG_RUNTIME_DIR` two instances can still run. `--demo` runs alongside, without the socket.

## Dux calendar backend

Antarctical feeds its events to the [Dux](https://github.com/duxlabs-dev) desktop shell (a DankMaterialShell fork) by serving the socket of Dux Calendar's `dankcal` daemon, read-only. Dux's bar clock, dashboard and calendar then show Antarctical's calendars.

- **Socket:** `$XDG_RUNTIME_DIR/dankcal-<pid>.sock` (mode 0600, created at start, removed on quit), or `$DANKCAL_SOCKET` when set. Set `DANKCAL_SOCKET` for both Dux and Antarctical, or for neither. Without a private `XDG_RUNTIME_DIR` there is no socket.
- **Dux setting:** set the calendar backend to `dankcal` (Settings → Time & Weather, or `"calendarBackend": "dankcal"` in Dux's `settings.json`). Dux only looks for the socket for about a minute after it starts or loses the connection, so start Antarctical with the session (`--hidden`), before Dux. On `auto` Dux looks only once, at start.
- **What Dux sees:** every calendar, read-only, with `hidden` following the sidebar checkboxes; events of the months Antarctical has loaded. When Dux asks for months that are not loaded, Antarctical answers with what it has, fetches them, and tells Dux to reload. Tasks are always empty; creating, editing or deleting from Dux is refused (`read-only`).
- **Declined events:** with **Show events I declined** on, they are served under a grey calendar `<calendar> (declined)` (id `<calendar id>#declined`), listed only while it has events. It is hidden when its calendar is.
- **Demo mode:** `--demo` never serves the socket, so Dux never sees generated events.
- **Real dankcal:** if a Dux Calendar daemon already runs, Antarctical warns that Dux may connect to either; if it owns `$DANKCAL_SOCKET`, Antarctical runs without a socket.

## Next-event bar widget

`dux-plugin/` is a Dux bar widget (id `antarcticalNextEvent`) that shows the current or next timed event today in its calendar's colour: `now`, `in 12m`, or the start time. All-day and declined events are skipped, and the widget hides when nothing is left today. Clicking it opens Antarctical (by store path when built with Nix, otherwise from `PATH`); from five minutes before an event with a video call until it ends, a camera button joins the call (`xdg-open`). It reads Dux's own calendar data, so it needs the `dankcal` backend above.

```sh
nix build github:korora-tech/antarctical#dux-plugin   # the plugin directory, for ~/.config/dms/plugins/antarcticalNextEvent
```

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
