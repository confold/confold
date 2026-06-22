# Chocolatey

Published to **community.chocolatey.org** as `confold` (packed + pushed by
`publish-chocolatey.yml` on release).

## Install

```powershell
choco install confold
```

## Files

- `confold.nuspec` — package metadata.
- `tools/chocolateyinstall.ps1` — downloads the official `-setup.exe` (NSIS), verifies its
  SHA256, and installs it silently (`/S`). Chocolatey's auto-uninstaller picks up the
  Add/Remove Programs entry NSIS registers, so no uninstall script is needed.
- `tools/VERIFICATION.txt` — moderator verification (download source + checksum).
- `tools/LICENSE.txt` — copy of the project's Apache-2.0 license (required by moderation).

URL, checksum, version and release-notes link are bumped by `scripts/bump-packaging.sh`.

## Notes

`push.chocolatey.org` intermittently returns 504; the workflow retries with backoff and treats a
504-then-403 (duplicate) as success. Moderation can take days — `verify-distribution.yml` tracks
when the package goes live.
