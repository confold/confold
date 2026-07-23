#!/usr/bin/env bash
set -euo pipefail

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
CHECKER="${ROOT}/scripts/check-stale-version-refs.sh"
FIXTURE=$(mktemp -d)
trap 'rm -rf "$FIXTURE"' EXIT

OLD_VERSION="9.8.7"

mkdir -p "$FIXTURE/confold-app" "$FIXTURE/src"
cat > "$FIXTURE/confold-app/pnpm-lock.yaml" <<EOF
packages:
  dependency@${OLD_VERSION}: {}
EOF
cat > "$FIXTURE/confold-app/package-lock.json" <<EOF
{"packages":{"dependency":{"version":"${OLD_VERSION}"}}}
EOF

if ! "$CHECKER" "$OLD_VERSION" "$FIXTURE"; then
  echo "Lockfile dependency versions must not be reported as stale application versions." >&2
  exit 1
fi

cat > "$FIXTURE/src/version.ts" <<EOF
export const version = "${OLD_VERSION}";
EOF

set +e
OUTPUT=$("$CHECKER" "$OLD_VERSION" "$FIXTURE" 2>&1)
STATUS=$?
set -e

if [[ $STATUS -eq 0 ]]; then
  echo "A stale application version must fail the check." >&2
  exit 1
fi

if [[ "$OUTPUT" != *"src/version.ts"* ]]; then
  echo "The stale application file must be included in the diagnostic." >&2
  exit 1
fi

echo "stale version reference checks passed"
