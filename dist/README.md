# Confold — packaging manifests

Package-manager manifests for distributing Confold. Confold is a Tauri app, so each channel
consumes one of the **installer** bundles produced by `.github/workflows/release.yml` (there is
no portable archive):

| Channel | Bundle used | Repo / target |
|---|---|---|
| Homebrew **cask** (macOS) | `Confold_<v>_aarch64.dmg` / `Confold_<v>_x64.dmg` | `confold/homebrew-confold` → `Casks/confold.rb` |
| Homebrew **formula** (Linux) | `Confold_<v>_amd64.AppImage` | `confold/homebrew-confold` → `Formula/confold.rb` |
| **Scoop** (Windows) | `Confold_<v>_x64-setup.exe` (extracted as 7z) | `confold/scoop-confold` → `bucket/confold.json` |
| **winget** (Windows) | `Confold_<v>_x64_en-US.msi` | PR to `microsoft/winget-pkgs` via the `juanyque/winget-pkgs` fork |
| **Chocolatey** (Windows) | `Confold_<v>_x64-setup.exe` (silent `/S`) | `community.chocolatey.org` |
| Linux direct | `.deb` / `.rpm` / `.AppImage` | release assets (linked from the site/README) |

## Cutting a release

1. Push a `v*` tag → `release.yml` builds all bundles into a **draft** GitHub release.
2. After every platform build succeeds, the workflow runs `./scripts/bump-packaging.sh <version>`.
   It reads the draft's digests from the GitHub API, validates the winget manifests, rewrites all
   package manifests and website download links, then commits those generated changes to `main`.
3. Review the workflow's packaging commit and the draft release assets.
4. Publish the draft release. On `release: published`:
   - `publish-distributions.yml` pushes the cask+formula to the tap, the manifest to the Scoop
     bucket, and opens a winget PR.
   - `publish-chocolatey.yml` packs and pushes to Chocolatey.
5. `verify-distribution.yml` runs daily and keeps a `[release-watch]` tracking issue open until
   winget + Chocolatey clear their moderation queues.

Secrets required (repo settings): `DIST_GITHUB_TOKEN` (classic PAT, `repo` scope — write to the
tap, bucket, and winget fork) and `CHOCOLATEY_API_KEY`.
