# Confold Homebrew Tap

[Homebrew](https://brew.sh) tap for [Confold](https://confold.com) — a fast, keyboard-driven folder & file
**comparison, migrate and sync** tool for macOS, Linux and Windows.

## Install

**macOS** — installs `Confold.app`:

```bash
brew tap confold/confold
brew install --cask confold
```

**Linux** — installs the Confold AppImage with a `confold` launcher on your `PATH`:

```bash
brew tap confold/confold
brew install confold
```

## What's in this tap

| File | Description |
|---|---|
| `Casks/confold.rb` | macOS app bundle — installs `Confold.app` from the official `.dmg` (Apple Silicon + Intel). |
| `Formula/confold.rb` | Linux formula — installs the AppImage into `libexec` and a `confold` launcher (extract-and-run mode, no system FUSE needed). |

These files are updated automatically from the main repo on each release, via
[`publish-distributions.yml`](https://github.com/confold/confold/blob/main/.github/workflows/publish-distributions.yml).

## Notes

- The macOS build is unsigned (code-signing is on the roadmap) — on first launch, right-click → **Open**, or
  allow it under *System Settings → Privacy & Security*.
- The Linux formula is best-effort (Confold is a GUI app with no portable CLI binary); the `.deb`, `.rpm`
  and `.AppImage` on the [releases page](https://github.com/confold/confold/releases/latest) are first-class too.

---

Confold is free and open source · [Apache-2.0](https://github.com/confold/confold/blob/main/LICENSE) ·
[confold.com](https://confold.com) · [github.com/confold/confold](https://github.com/confold/confold)
