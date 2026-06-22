# winget

Submitted to **`microsoft/winget-pkgs`** as `Confold.Confold`, via an automated PR from the
`juanyque/winget-pkgs` fork (`publish-distributions.yml` on release).

## Install

```powershell
winget install Confold.Confold
```

## Files

`manifests/c/Confold/Confold/<version>/`:

- `Confold.Confold.yaml` — version manifest.
- `Confold.Confold.installer.yaml` — the `.msi` (WiX) installer, `InstallerType: wix`,
  `Scope: perMachine`. winget reads the ProductCode from the MSI itself at install time, so it is
  not duplicated here. No `VCRedist` dependency — Tauri's installer handles the WebView2 runtime.
- `Confold.Confold.locale.en-US.yaml` — publisher/description/tags.

`scripts/bump-packaging.sh` copies the previous version folder to a new one and rewrites the
version, MSI URL and `InstallerSha256`.

## Notes

- The CLA is a **one-time** signature; the `Needs-CLA` label is transient noise on every PR — the
  `license/cla` commit status is authoritative (`verify-distribution.yml` keys off that).
- winget moderation can take days; `verify-distribution.yml` tracks it.
