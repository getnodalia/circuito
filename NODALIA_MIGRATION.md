# Nodalia migration: Circuito

Category: new app, built for Nodalia from the start (no upstream, no inherited code).
Status: first version on a pull request, 2026-11-26.

## Current state

See `README.md` for what runs. CI calls Foundation's `rust-gate`, `git-sources` and `agents-check`
workflows, pinned by commit, plus the release-notes check.

## Desired state

Engineering suite look through `nodalia_theme::engineering`, shared with Forja, Torno and Vortia;
release workflows (`release-auto`, `release-linux`, `release-macos` with a DMG window in Circuito's
colour, `release-windows`) through Foundation's `release-tag` and `release-publish`; the portrait
app icon; website page getnodalia.com/circuito; a Nodalia Launcher catalog entry.

## Inherited dependencies still pending

None.

## Reusable Nodalia components

- `nodalia-theme` (egui, `engineering` feature) from nodalia-design: palette, islands, ribbon,
  properties panel, window controls, icons, About page. Circuito's accent is `Accent::Circuito`.
- `nodalia-update`: the in-app update check.

## NDL work pending

- The accent lands in nodalia-design first; until then the app runs on the default accent.
- The app icon is a portrait of a historical figure (see `ATTRIBUTION.md` when it lands).

## Missing tests

- The views are checked by hand with screenshots; a headless render test of the main screens
  would catch layout regressions.
