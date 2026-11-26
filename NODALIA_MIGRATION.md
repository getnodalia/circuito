# Nodalia migration: Circuito

Category: new app, built for Nodalia from the start (no upstream, no inherited code).
Status: first version on a pull request, 2026-11-26.

## Current state

See `README.md` for what runs. CI calls Foundation's `rust-gate`, `git-sources` and `agents-check`
workflows, pinned by commit, plus the release-notes check.

Releases: `release-auto.yml` tags a merged version bump through Foundation's `release-tag`,
`release-linux.yml` (AppImage with its `.zsync`, `.deb`, `.rpm`, `.tar.gz`), `release-macos.yml`
(universal DMG with its own install window in Circuito's colour, ad hoc signed, plus the command as a
zip) and `release-windows.yml` (MSI and portable zip, x64, unsigned unless signing secrets are set)
build and attach the packages, and Foundation's `release-publish` publishes. File names follow
`<app>-<version>-<os>-<arch>` so the Nodalia Launcher can pick them. The scripts are in
`packaging/`. Only the Linux packaging has been run locally; the macOS and Windows jobs and the
workflows themselves have not run on GitHub yet.

## Desired state

Engineering suite look through `nodalia_theme::engineering`, shared with Forja, Torno and Vortia;
the portrait app icon; website page getnodalia.com/circuito; a Nodalia Launcher catalog entry.

## Inherited dependencies still pending

None.

## Reusable Nodalia components

- `nodalia-theme` (egui, `engineering` feature) from nodalia-design: palette, islands, ribbon,
  properties panel, window controls, icons, About page. Circuito's accent is `Accent::Circuito`.
- `nodalia-update`: the in-app update check.

## NDL work pending

- The accent lands in nodalia-design first; until then the app runs on the default accent.
- The app icon is a portrait of a historical figure. Until it lands the app draws a placeholder
  mark in code (`apps/circuito/src/icon.rs`), recorded in `ATTRIBUTION.md`.
- Not verified yet: the Linux global menu on a real shell, NDL window controls on macOS and
  Windows, Wayland.
- Packaging follow-ups: `circuito-app.exe` has no icon resource embedded (the Start menu shortcut and
  file type use the `.ico` shipped in the MSI, Explorer shows a generic icon for the exe itself); the
  macOS app does not handle the Finder open-file event yet, so a double-click on a `.circuito` file
  starts the app without opening it; the DMG's picture lives in `packaging/macos/dmg/draw_background.py`
  until nodalia-design has a Circuito entry in its DMG background table; there is no Flatpak,
  Arch or FreeBSD package and no `packaging-lint` workflow yet.

## Missing tests

- The views are checked by hand with screenshots; a headless render test of the main screens
  would catch layout regressions. The app's view-model (documents, findings, plots, settings,
  i18n tables, command and menu catalogue) is covered by unit tests.
