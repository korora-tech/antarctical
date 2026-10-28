# Antarctical artwork

The app and website share these assets:

- `antarctical-penguin.svg`: detailed mascot for the website and sign-in screen.
- `antarctical-app-icon.svg`: compact penguin for app headers, the website header and favicon, and the installed Linux desktop icon.
- `antarctical-app-icon.png`: 512 px export of the compact SVG. The system tray decodes it into the ARGB pixmaps required by StatusNotifierItem. It contains no date or changing badge.

Both SVGs have transparent backgrounds and editable, named groups. They contain no embedded bitmap images, fonts, scripts, or external resources. Edit the SVGs here; the website imports them directly and GPUI embeds them in the binary. The Nix package installs the desktop icons under `share/icons/hicolor` using the app ID `korora.tech.Antarctical`.

After changing the compact SVG, regenerate the PNG with librsvg:

```sh
nix shell nixpkgs#librsvg -c rsvg-convert \
  --width 512 --height 512 \
  --output data/branding/antarctical-app-icon.png \
  data/branding/antarctical-app-icon.svg
```
