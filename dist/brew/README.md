# Homebrew

Published to the tap **`confold/homebrew-confold`** (auto-pushed by
`publish-distributions.yml` on release).

## Install

```sh
brew tap confold/confold
brew install --cask confold      # macOS (installs Confold.app from the .dmg)
brew install confold             # Linux (AppImage-backed formula)
```

## Files

- `Casks/confold.rb` — macOS cask. Installs `Confold.app` from the per-arch `.dmg`
  (`on_arm` / `on_intel`).
- `Formula/confold.rb` — Linux formula. Confold has no portable CLI binary, so the formula
  installs the `.AppImage` into `libexec` and writes a `confold` launcher that runs it in
  `APPIMAGE_EXTRACT_AND_RUN` mode (no system FUSE required). This path is best-effort — verify
  on a real Linuxbrew host after each bump.

Both files are version-bumped by `scripts/bump-packaging.sh`.
