#!/usr/bin/env bash
set -euo pipefail

MANIFEST_DIR="${1:-}"
EXPECTED_VERSION="${2:-}"
SCHEMA_VERSION="1.12.0"
PUBLISHER="confold"

if [[ -z "$MANIFEST_DIR" || -z "$EXPECTED_VERSION" ]]; then
  echo "Usage: $0 <manifest-directory> <package-version>" >&2
  exit 1
fi

fail() {
  echo "Error: $1" >&2
  exit 1
}

validate_file() {
  local file="$1"
  local schema_type="$2"
  local manifest_type="$3"

  [[ -f "$file" ]] || fail "missing winget manifest: $file"
  grep -Fqx "PackageIdentifier: Confold.Confold" "$file" \
    || fail "$file has an unexpected PackageIdentifier"
  grep -Fqx "PackageVersion: ${EXPECTED_VERSION}" "$file" \
    || fail "$file does not target package version ${EXPECTED_VERSION}"
  grep -Fqx "ManifestType: ${manifest_type}" "$file" \
    || fail "$file has an unexpected ManifestType"
  grep -Fqx "ManifestVersion: ${SCHEMA_VERSION}" "$file" \
    || fail "$file does not use ManifestVersion ${SCHEMA_VERSION}"
  grep -Fqx "# yaml-language-server: \$schema=https://aka.ms/winget-manifest.${schema_type}.${SCHEMA_VERSION}.schema.json" "$file" \
    || fail "$file does not reference the ${SCHEMA_VERSION} ${schema_type} schema"
}

validate_file \
  "$MANIFEST_DIR/Confold.Confold.installer.yaml" \
  "installer" \
  "installer"
validate_file \
  "$MANIFEST_DIR/Confold.Confold.locale.en-US.yaml" \
  "defaultLocale" \
  "defaultLocale"
validate_file \
  "$MANIFEST_DIR/Confold.Confold.yaml" \
  "version" \
  "version"

grep -Fqx "Publisher: ${PUBLISHER}" \
  "$MANIFEST_DIR/Confold.Confold.locale.en-US.yaml" \
  || fail "default locale Publisher must be ${PUBLISHER}"

echo "winget manifests are valid for Confold ${EXPECTED_VERSION}"
