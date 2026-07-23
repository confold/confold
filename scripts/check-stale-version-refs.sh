#!/usr/bin/env bash
set -euo pipefail

VERSION="${1:-}"
SEARCH_ROOT="${2:-.}"

if [[ -z "$VERSION" ]]; then
  echo "Usage: $0 <version> [search-root]" >&2
  exit 2
fi

STALE=$(grep -rF "$VERSION" \
  --include="*.rs" --include="*.ts" --include="*.tsx" \
  --include="*.toml" --include="*.json" --include="*.md" \
  --include="*.html" --include="*.sh" --include="*.rb" \
  --include="*.yml" --include="*.yaml" --include="*.txt" \
  --exclude="CHANGELOG.md" \
  --exclude="*.lock" \
  --exclude="*-lock.*" \
  --exclude-dir=node_modules \
  --exclude-dir=target \
  --exclude-dir=dist \
  --exclude-dir=.git \
  --exclude-dir=graphify-out \
  "$SEARCH_ROOT" 2>/dev/null || true)

if [[ -n "$STALE" ]]; then
  printf "%s\n" "$STALE"
  exit 1
fi
