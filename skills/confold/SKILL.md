---
name: confold
description: Use Confold's validated CLI protocol to compare two Markdown or plain-text documents, optionally against a common base, and prepare safe AI-assisted merge proposals with stale-input checks and explicit output confirmation. Use for semantic comparison, semantic merge, reconciling diverged prose, consolidating document versions, or preserving intent across restructured documentation. Do not use for source code, structured configuration, binary files, or folder-wide replica synchronization.
---

# Confold

Use Confold as the deterministic I/O and validation boundary. Perform semantic judgment in the agent,
but never edit either compared input directly.

## Safety invariants

- Run `confold semantic prepare`, `review`, and `apply`; do not reproduce their file operations.
- Treat bundle and proposal files as temporary exchange artifacts, not durable project state.
- Never modify, replace, delete, stage, or commit an input document through this workflow.
- Write only to a separate output path that does not exist.
- Stop on unsupported protocol, stale input, invalid proposal, binary content, or uncertainty.
- Invoke `semantic apply` only after showing the review and receiving explicit user confirmation.

## Workflow

### 1. Check compatibility

Resolve `confold` from the active environment. Do not hard-code a private installation path.

```bash
confold capabilities --format json
```

Require semantic protocol version `1`. If the executable is missing or incompatible, explain the
requirement and stop without writing.

### 2. Resolve the inputs

Use exactly two variants, with an optional common base:

- `left`: first current variant;
- `right`: second current variant;
- `base`: last known common version, when available.

Show the resolved paths and roles before analysis. Confold v1 supports prose-oriented UTF-8 files with
extensions reported by `capabilities`. Do not rename an unsupported file merely to bypass validation.

### 3. Prepare an immutable bundle

Create a temporary directory and run:

```bash
confold semantic prepare \
  --left LEFT_PATH \
  --right RIGHT_PATH \
  --base BASE_PATH \
  --output TEMP_DIR/bundle.json
```

Omit `--base` for a two-way comparison. Read the resulting bundle and honor its `fast_path`:

- `byte_identical`: report that no semantic merge is needed.
- `formatting_only`: report the byte-level formatting difference; do not invoke semantic judgment.
- `prefer_left` or `prefer_right`: one side equals the base, so propose the changed side without
  rewriting its content.
- `needs_semantic_analysis`: continue with structural and intent analysis.

### 4. Analyze meaning

For each input:

1. Map heading paths and nearby introductory text when Markdown structure exists.
2. Identify shared intent, unique contributions, changed decisions, direct conflicts, and uncertainty.
3. Use the base to distinguish inherited material from changes made independently on each side.
4. Classify each material contribution as `preserved`, `already_present`, `superseded`, `omitted`, or
   `uncertain`.
5. Preserve both variants when confidence is insufficient. Never silently choose newer, larger, or
   longer content.

Do not fabricate missing facts or smooth over contradictory requirements. Explain every omission and
superseded contribution.

### 5. Write only a proposal

Read [the protocol reference](references/protocol.md) when constructing the JSON. Copy
`operation_id` from the bundle exactly.

- Use `equivalent` when meaning is the same and no output should be written.
- Use `prefer_left` or `prefer_right` when one complete variant should become the separate output.
- Use `merged` with the full proposed text when both sides contribute.
- Use `uncertain` with no result whenever unresolved meaning remains.

Write the proposal to `TEMP_DIR/proposal.json`. Do not write merged prose anywhere else yet.

### 6. Review deterministically

```bash
confold semantic review \
  --bundle TEMP_DIR/bundle.json \
  --proposal TEMP_DIR/proposal.json \
  --format json
```

Present:

- verdict and summary;
- contribution dispositions;
- every warning and uncertainty;
- the full deterministic diff from each input to the proposed result;
- whether the proposal is applicable.

If review fails, do not repair the bundle manually. Rerun `prepare` for stale inputs, or correct only
the proposal for a schema/logic error and review again.

### 7. Apply only after confirmation

Ask the user to confirm the reviewed result and choose a separate output path. Then run:

```bash
confold semantic apply \
  --bundle TEMP_DIR/bundle.json \
  --proposal TEMP_DIR/proposal.json \
  --output NEW_OUTPUT_PATH \
  --format json
```

Report the canonical output path and SHA-256 returned by Confold. Leave replacement, staging, commit,
and publication decisions to the user or to a separately authorized workflow.

## Failure handling

- If either input changes after `prepare`, discard the proposal and restart from a new bundle.
- If the output already exists, choose another path; never remove or overwrite it automatically.
- If `review` reports `applicable: false`, do not call `apply`.
- If the agent cannot account for a contribution, use `uncertain` and ask the user for direction.
- If source code or structured data needs merging, stop and use a resolver designed for that format.
