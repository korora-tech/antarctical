# App screenshots

These are captures of the native Antarctical app running with `--demo`, not mockups.
They use only the generated accounts and events in `src/demo.rs`.

- App source: commit `31076f433ce1f39f8618609284caa540a497590c`.
- Captured on 28 October 2026 at 1278 × 820, using X11 and software Vulkan rendering.
- Stored as lossless WebP, without retouching or replacing any UI.
- Views: month, week, day with event details, and agenda.

To refresh them, run `nix run . -- --demo` from the repository root. Use
`m`, `w`, `d`, and `a` to select each view, scroll the time grid to the working
day, and use your desktop's window capture tool. Open an event for the day-view
capture. Keep the generated account labels and the “Demo data” indicator visible.
Replace the corresponding lossless WebP files and update the source commit and
capture date above. Never capture a signed-in account for these assets.
