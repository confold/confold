# Semantic protocol v1

Load this reference when creating or troubleshooting a Confold semantic proposal.

## Commands

```text
confold capabilities --format json

confold semantic prepare \
  --left LEFT_PATH --right RIGHT_PATH [--base BASE_PATH] \
  --output BUNDLE_PATH

confold semantic review \
  --bundle BUNDLE_PATH --proposal PROPOSAL_PATH --format json

confold semantic apply \
  --bundle BUNDLE_PATH --proposal PROPOSAL_PATH \
  --output NEW_OUTPUT_PATH --format json
```

Every created path must be new. Confold never overwrites a bundle or merged output.

## Proposal schema

```json
{
  "schema_version": 1,
  "operation_id": "copy exactly from bundle",
  "verdict": "equivalent | prefer_left | prefer_right | merged | uncertain",
  "summary": "non-empty explanation",
  "contributions": [
    {
      "source": "left | right | base",
      "intent": "material idea or requirement",
      "disposition": "preserved | already_present | superseded | omitted | uncertain"
    }
  ],
  "warnings": ["remaining risk or ambiguity"],
  "result": "full merged text or null"
}
```

Verdict rules:

| Verdict | `result` | Applicable |
|---|---|---|
| `equivalent` | `null` | no |
| `prefer_left` | `null` | yes; Confold uses the captured left content |
| `prefer_right` | `null` | yes; Confold uses the captured right content |
| `merged` | full text | yes |
| `uncertain` | `null` | no |

Do not include result text for any verdict except `merged`.

## Bundle facts

The bundle contains:

- `schema_version` and `proposal_schema_version`;
- opaque `operation_id`;
- canonical paths and roles;
- SHA-256, byte length, EOL style, final-newline state, and bounded UTF-8 content;
- `fast_path` classification.

Do not edit the bundle. Review rejects inconsistent snapshot metadata, roles, hashes, or fast paths.

## Review and apply guarantees

Both commands:

- require protocol version 1;
- require matching operation IDs;
- validate verdict/result combinations and contribution sources;
- re-read every input and reject stale content.

Review is read-only and emits unified diffs. Apply repeats validation and atomically creates a new
output. It rejects an existing output and any path that would replace an input.

CLI failures exit with code `2` and write an `error:` message to stderr.
