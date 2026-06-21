#!/usr/bin/env bash
# Local S3-compatible server (pure Rust, NO Docker) for testing Confold's S3 source end to end.
# Runs s3s + s3s-fs over a host directory via a cargo example — no MinIO, no Docker daemon needed.
#
# Usage:  ./scripts/s3-demo.sh [dir] [port]
#   dir   host directory to serve   (default: /tmp/confold-s3, created + seeded if empty)
#   port  local port to listen on   (default: 4566)
#
# Then in Confold, add an "S3" source and fill in the printed endpoint / region / bucket / keys.
# Stop with Ctrl-C.
set -euo pipefail
cd "$(dirname "$0")/.."
# confold-s3 is excluded from the workspace (see its Cargo.toml), so target it by manifest path —
# `-p confold-s3` from the workspace root can't find a non-member.
exec cargo run --quiet --manifest-path crates/confold-s3/Cargo.toml --example s3-demo -- "$@"
