# Changelog

All notable changes to Confold are documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.5.1] — unreleased

### Fixed
- macOS bundles are now ad-hoc signed, so Gatekeeper shows the standard "unidentified developer"
  prompt with an **Open Anyway** button in System Settings → Privacy & Security, instead of the
  "damaged and can't be opened" dead-end on Sonoma and later.

## [0.5.0] — 2026-06-22

First public release. (Versions 0.1–0.4 were internal development milestones.)

### Added
- **Three modes, one engine** — Compare, Migrate, and Sync over a shared compare/reconcile engine.
- **Sources** — local filesystem, SFTP (pure-Rust `russh`), and S3 / S3-compatible (`object_store`:
  AWS, MinIO, Cloudflare R2 …), behind one capability-gated plugin interface.
- **Compare** — lazy, virtualized folder tree with live streaming verdicts, status counts and filter;
  word/character-level side-by-side diffs with hunk navigation; hunks-only view + hex view for large files.
- **Migrate** — dry-run plan with per-item checkboxes, streaming cancellable apply, and a verified
  **move** (delete origin only after re-verifying every byte landed, all-or-nothing).
- **Sync** — bidirectional reconciliation with conflict resolution, on the Migrate engine.
- Cross-platform desktop app (Tauri v2 + Svelte 5) for Linux, macOS and Windows.

[Unreleased]: https://github.com/confold/confold/compare/v0.5.1...HEAD
[0.5.1]: https://github.com/confold/confold/compare/v0.5.0...v0.5.1
[0.5.0]: https://github.com/confold/confold/releases/tag/v0.5.0
