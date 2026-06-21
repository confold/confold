#!/usr/bin/env bash
# Local SFTP server for testing Confold's SFTP source, end to end.
#
# NOTE: Confold speaks **SFTP (over SSH)**, not classic plaintext FTP — so this is an SFTP server.
# Runs the `atmoz/sftp` Docker image with user `ftp` / password `ftp`, serving a host directory.
#
# Usage:  ./scripts/sftp-demo.sh [dir] [port]
#   dir   host directory to serve   (default: /tmp/ftp, created + seeded if empty)
#   port  local port to listen on   (default: 2222; avoids clashing with system sshd on 22)
#
# Then in Confold, paste this URL into a source picker (Origen or Destino):
#     sftp://ftp@localhost:<port>/data        (password: ftp)
# The served directory is mounted at /data inside the SFTP chroot.
#
# Stop with Ctrl-C (the container is --rm, so it cleans itself up).
set -euo pipefail

DIR="${1:-/tmp/ftp}"
PORT="${2:-2222}"

if ! command -v docker >/dev/null 2>&1; then
  echo "error: docker not found. Install Docker (or start the daemon) — this demo uses the atmoz/sftp image." >&2
  exit 1
fi
if ! docker info >/dev/null 2>&1; then
  echo "error: the Docker daemon isn't running. Start Docker Desktop and retry." >&2
  exit 1
fi

mkdir -p "$DIR"
# Seed a small tree the first time, so there's something to compare against.
if [ -z "$(ls -A "$DIR" 2>/dev/null)" ]; then
  printf 'hello from the SFTP demo\n' > "$DIR/readme.txt"
  printf '{\n  "env": "demo",\n  "n": 1\n}\n' > "$DIR/config.json"
  mkdir -p "$DIR/sub"
  printf 'nested file\n' > "$DIR/sub/note.txt"
  echo "Seeded demo files into $DIR"
fi

echo "──────────────────────────────────────────────────────────────"
echo " SFTP demo server"
echo "   URL  : sftp://ftp@localhost:$PORT/data   (password: ftp)"
echo "   Serves: $DIR  →  /data"
echo "   Paste that URL into a Confold source picker; Ctrl-C to stop."
echo "──────────────────────────────────────────────────────────────"

# user:pass:uid  — uid 1001 keeps the mounted files writable for sync/migrate tests.
exec docker run --rm -p "$PORT:22" -v "$DIR:/home/ftp/data" atmoz/sftp ftp:ftp:1001
