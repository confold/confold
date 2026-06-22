# Confold Scoop Bucket

[Scoop](https://scoop.sh) bucket for [Confold](https://confold.com) (Windows) — a fast, keyboard-driven
folder & file **comparison, migrate and sync** tool.

## Install

```powershell
scoop bucket add confold https://github.com/confold/scoop-confold
scoop install confold
```

## How it works

Confold is a Tauri app, so it ships as an NSIS installer rather than a portable archive. NSIS installers are
7-Zip–extractable, so `bucket/confold.json` points Scoop at the `-setup.exe` with a `#/dl.7z` URL fragment —
Scoop **extracts** the app and shims `Confold.exe`, keeping the install self-contained and uninstallable the
Scoop way. `checkver` + `autoupdate` let `scoop update` (and maintainers' excavator bots) pick up new
releases automatically.

## What's in this bucket

| File | Description |
|---|---|
| `bucket/confold.json` | Scoop manifest — version, hash and download URL for the Windows build. |

Updated automatically from the main repo on each release, via
[`publish-distributions.yml`](https://github.com/confold/confold/blob/main/.github/workflows/publish-distributions.yml).

## Notes

The Windows build is unsigned (code-signing is on the roadmap), so SmartScreen may warn on first run. Scoop
installs from your user path are generally unaffected.

---

Confold is free and open source · [Apache-2.0](https://github.com/confold/confold/blob/main/LICENSE) ·
[confold.com](https://confold.com) · [github.com/confold/confold](https://github.com/confold/confold)
