# Confold — Agent Guide

> Agent-facing operating guide for this repository. Read first.

## What this project is

A **free, open-source, cross-platform folder-compare and controlled-synchronization tool**,
independently implemented (clean-room; no third-party GPL code reused). It delivers a fast, proven
workflow — metadata triage (name/date/size) plus deep byte-by-byte and partial large-file content
comparison — to Linux, macOS, and Windows, and extends it
toward **controlled synchronization across environments** (local now; SMB/S3 later) with an interactive
GUI, and eventually AI-assisted **semantic** reconciliation.

## Current phase

**Phase 1 (MVP) implemented.** A headless compare engine (`confold-core`) over a pluggable VFS source
abstraction (`confold-vfs`), plus a CLI (`confold`). Local filesystem only; no GUI yet. Next: **Phase 2** — the
interactive compare-and-sync GUI (web UI + Tauri).

Application code is written against an **approved spec** (see Workflow below).

## Documentation

- **Publishable docs live in this repository**: `README.md` (overview + CLI usage) and per-crate
  `README.md`s. Clean public docs (user guide, architecture, CONTRIBUTING) are authored as the project
  approaches publication.
- **Internal/dev design docs** — feasibility analysis, decision log, roadmap, and phase specs — are kept
  **privately in the maintainer's notes during development** and are intentionally **not** in this
  repository. They are the source of truth for *how* and *why*; ask the maintainer if you need them.

## How to work in this repo

**Method: Spec-Driven Development (SDD) + autonomous mode.**

- **Spec-first.** Every phase/feature starts as a written spec (requirements → design → task breakdown),
  maintained privately. The spec is the source of truth and the review gate; update it if reality diverges.
- **Autonomous mode.** Once the maintainer approves a spec, advance autonomously through its tasks (write
  code + tests, run them, iterate) **without per-step approval.** Report progress; stop only at a **hard
  intervention point**:
  1. Spec/architecture approval before implementing a new phase.
  2. Git — the maintainer commits/pushes; **never commit or push autonomously.**
  3. Outward/irreversible — publishing the repo, choosing the final name, any upstream contribution,
     publishing releases.
  4. External infra/credentials — Proxmox, cloud accounts, secrets.
  5. A new unmade decision.
  Within those bounds: **progress over permission.**
- **Clean-room discipline.** Any third-party GPL tool is read as a *behavioral reference + test oracle
  only*. Never copy GPL-licensed code into this Apache-2.0 tree. Contributions to upstream tools flow
  outward only.
- **Reuse over reinvent**, with license compatibility (permissive/Apache; avoid GPL/AGPL deps).
- **Use sub-agents** for broad research/exploration to keep the main context clean.

## Quality gates

```
cargo test --all     # unit + integration (incl. differential checks vs native diff/cmp)
cargo lint           # clippy with -D warnings (alias in .cargo/config.toml)
cargo fmt --all      # format
```

## Language

All shared content is in **English**: source code, comments, docs, commit messages, PR descriptions.

## Git workflow

Personal project. Work on the default branch unless a reason to branch arises. **The maintainer handles
all commits and pushes** — do not commit or push autonomously.

## Reference checkout

Upstream reference tools are consulted read-only as behavioral references and test oracles; their
sources are not committed here.

## Future work (noted)

- **AI SKILL for driving Confold** — once a usable tool exists, a companion skill teaching agents to
  use it (folder consolidation, dedup, deep diff). Out of scope until the tool exists.
- **Semantic (AI-assisted) synchronization** — reconcile by intent, not just bytes/lines (a pluggable
  resolver layer). A key differentiator; later phase.
