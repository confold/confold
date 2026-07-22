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

**v0.6.0 shipped (public release).** Confold is a complete, cross-platform folder-compare and sync tool
with a Tauri v2 + Svelte 5 desktop GUI, a headless CLI, and a layered engine crate architecture.

**Three modes on one engine:** Compare (lazy/incremental folder tree, metadata + byte-level content
compare, side-by-side/hex/image diff), Migrate (reconcile destination to match origin, with verified
move semantics — all-or-nothing delete after full byte re-verify), and Sync (bidirectional with
conflict resolution).

**Sources:** Local filesystem, SFTP (pure-Rust `russh`), and S3/S3-compatible (pure-Rust
`object_store`) — all behind a uniform plugin interface (`SourceKind` registry, Level 1 capability-gated
descriptors). Adding S3 required zero frontend changes, proving the plugin model.

**Distribution:** GitHub releases (`github.com/confold/confold`), web (`confold.com`, Cloudflare Pages),
Homebrew + Scoop + Chocolatey + winget. Ad-hoc macOS signing (no Apple Developer account needed).

**Semantic protocol:** `confold-semantic`, the `confold semantic prepare/review/apply` CLI and the
public `skills/confold` workflow provide a validated proposal boundary for two-way and three-way prose
reconciliation. Apply creates a separate output and rejects stale inputs or existing destinations.

**Next:** connect this protocol to desktop file actions and unresolved Sync conflicts. Later: deeper
sources (SMB, NFS, WebDAV), structured resolvers and baseline-aware replica reconciliation.

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

- **Desktop semantic actions**: expose the validated proposal/review/apply flow from the side-by-side
  viewer without duplicating its Rust safety boundary.
- **Semantic folder synchronization**: feed unresolved Sync conflicts through the resolver, backed by
  durable baselines and exact replica metadata.

## graphify

This project has a knowledge graph at graphify-out/ with god nodes, community structure, and cross-file relationships.

When the user types `/graphify`, use the installed graphify skill or instructions before doing anything else.

Rules:
- For codebase questions, first run `graphify query "<question>"` when graphify-out/graph.json exists. Use `graphify path "<A>" "<B>"` for relationships and `graphify explain "<concept>"` for focused concepts. These return a scoped subgraph, usually much smaller than GRAPH_REPORT.md or raw grep output.
- Dirty graphify-out/ files are expected after hooks or incremental updates; dirty graph files are not a reason to skip graphify. Only skip graphify if the task is about stale or incorrect graph output, or the user explicitly says not to use it.
- If graphify-out/wiki/index.md exists, use it for broad navigation instead of raw source browsing.
- Read graphify-out/GRAPH_REPORT.md only for broad architecture review or when query/path/explain do not surface enough context.
- After modifying code, run `graphify update .` to keep the graph current (AST-only, no API cost).
