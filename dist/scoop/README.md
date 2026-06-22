# Scoop

Published to the bucket **`confold/scoop-confold`** (auto-pushed by
`publish-distributions.yml` on release).

## Install

```powershell
scoop bucket add confold https://github.com/confold/scoop-confold
scoop install confold
```

## How it works

Tauri produces an NSIS installer, not a portable archive. NSIS installers are 7-Zip–extractable,
so `bucket/confold.json` points at the `-setup.exe` with a `#/dl.7z` URL fragment — Scoop then
**extracts** the app files (rather than running the installer) and shims `Confold.exe`. This keeps
the Scoop install self-contained and uninstallable the Scoop way.

⚠️ Best-effort: the extracted layout depends on how Tauri's NSIS packs the bundle. Verify
`scoop install confold` launches `Confold.exe` on a real Windows host after the first release and
after any Tauri/NSIS upgrade. Version + hash + url are bumped by `scripts/bump-packaging.sh`.
