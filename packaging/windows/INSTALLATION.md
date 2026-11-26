# Windows installation policy checks

New installations default to the current account under `Programs\Nodalia\Circuito`.
The first setup page offers installation for everyone. Scope is selected before duplicate
checks and directory costing. An update uses the existing context's remembered exact folder;
`INSTALLFOLDER` supplied explicitly is never replaced. It is a secure MSI property so the
chosen folder reaches the elevated installer. Silent machine updates pass `ALLUSERS=1`.
A cross-context copy is refused; changing context means uninstalling the old copy first.

App registration follows the selected context: the `.circuito` file type (Circuito is its
default app), "Open with" for `.cir` SPICE decks, the Start menu shortcut and an App Paths entry
for `circuito-app.exe`.

`installer-ui.wxs` and `install-policy.wxi` are the shared Nodalia installer snapshot as Pliego
carries it, with the app's names substituted. Edit the shared source and take the snapshot again
rather than changing them by hand. `circuito.wxs` is Circuito's own.

## Repeatable metadata tests

On Windows with WiX 5.0.2:

```powershell
./packaging/windows/test-install-policy.ps1 -OutputDirectory ./target/install-policy-tests
```

The script builds x64, x86 and arm64 packages with deliberately non-executable payloads from the
actual package source, then checks the compiled database (scope, folders, duplicate guards,
upgrade identity and the registry rows recorded in `install-policy.json`). The release packaging
script runs the same compiled checker before signing.

Never install or distribute the `NOT-INSTALLABLE.msi` fixtures as an app. Passing these checks
does not prove real installation, elevation, registry writes, shortcut behaviour, cancellation,
rollback or uninstall: the release workflow installs and removes the real MSI once on a clean
runner, and nothing here has been run on Windows by the author of this change.
