#!/usr/bin/env bash
# Generate a synthetic demo pair of folder trees for exercising `confold compare`.
# Doubles as demo data for the upcoming GUI. Re-runnable; writes nothing outside <target-dir>.
#
# Usage: scripts/gen-demo.sh <target-dir> [count] [big-lines]
#   <target-dir>  creates <target-dir>/left and <target-dir>/right
#   [count]       optional number of extra "bulk" files per side (default 0) to stress the
#                 tree / virtualization (e.g. 50 = small, 5000 = large).
#   [big-lines]   optional line count for a large TEXT file pair (text/large.txt, default 0 = none) to
#                 exercise the side-by-side virtualization — open it by double-clicking it in the tree.
set -euo pipefail

root="${1:?usage: gen-demo.sh <target-dir> [count] [big-lines]}"
count="${2:-0}"
biglines="${3:-0}"
L="$root/left"
R="$root/right"
mkdir -p "$L/sub" "$R/sub" "$L/only_left_tree/deep" "$R/only_right_tree"

# 1. Identical text file.
printf 'same content\n' >"$L/same.txt"; printf 'same content\n' >"$R/same.txt"

# 2. Same SIZE, different bytes — `--method size` calls it identical; `full`/`quick` call it different.
printf 'AAAA' >"$L/samesize.dat"; printf 'BBBB' >"$R/samesize.dat"

# 3. Different size — flagged different without reading contents.
printf 'left version\n' >"$L/readme.md"; printf 'right version, a bit longer\n' >"$R/readme.md"

# 4. Unique files (one per side).
printf 'only on the left\n'  >"$L/only_left.txt"
printf 'only on the right\n' >"$R/only_right.txt"

# 5. Nested: identical + different.
printf 'nested same\n' >"$L/sub/a.txt"; printf 'nested same\n' >"$R/sub/a.txt"
printf 'nested X\n'    >"$L/sub/b.txt"; printf 'nested Y\n'    >"$R/sub/b.txt"

# 6. Unique subtrees (enumerated recursively in the report).
printf 'deep left-only\n' >"$L/only_left_tree/deep/x.txt"
printf 'right-only\n'      >"$R/only_right_tree/y.txt"

# 7. Binary: identical + different.
head -c 4096 /dev/urandom >"$L/image.bin"; cp "$L/image.bin" "$R/image.bin"
head -c 2048 /dev/urandom >"$L/blob.bin";  head -c 2048 /dev/urandom >"$R/blob.bin"

# 8. Large identical file (8 MiB) — above the 4 MiB quick threshold, so `quick` samples it.
head -c 8388608 /dev/zero >"$L/big.bin"; cp "$L/big.bin" "$R/big.bin"

# 9. Noise to demo filters.
printf 'tmpL' >"$L/cache.tmp"; printf 'tmpR' >"$R/cache.tmp"

# 10. Rich multi-line text pairs under text/ — realistic, multi-hunk diffs for the side-by-side
#     view + copy-change/merge (P4). Each pair is "different", mixing replaced, inserted and deleted
#     lines plus word-level intra-line changes, with identical regions in between so alignment shows.
mkdir -p "$L/text" "$R/text"

# 10a. JSON config: changed values (word-level), a removed key, a grown array, an added key.
cat <<'EOF' >"$L/text/config.json"
{
  "name": "confold",
  "version": "0.3.0",
  "debug": false,
  "max_threads": 4,
  "retries": 3,
  "endpoints": [
    "https://api.example.com",
    "https://cdn.example.com"
  ]
}
EOF
cat <<'EOF' >"$R/text/config.json"
{
  "name": "confold",
  "version": "0.4.0",
  "debug": true,
  "max_threads": 8,
  "endpoints": [
    "https://api.example.com",
    "https://cdn.example.com",
    "https://backup.example.com"
  ],
  "timeout_ms": 5000
}
EOF

# 10b. Python source: a removed import, a new import, a reworked function body (word-level), a
#      whole added function (insert hunk) and an extra print line.
cat <<'EOF' >"$L/text/app.py"
import os
import sys
import json


def load(path):
    with open(path) as f:
        return json.load(f)


def main():
    cfg = load("config.json")
    print("threads:", cfg["max_threads"])


if __name__ == "__main__":
    main()
EOF
cat <<'EOF' >"$R/text/app.py"
import os
import json
from pathlib import Path


def load(path):
    with Path(path).open(encoding="utf-8") as f:
        return json.load(f)


def save(path, data):
    with open(path, "w") as f:
        json.dump(data, f, indent=2)


def main():
    cfg = load("config.json")
    print("threads:", cfg["max_threads"])
    print("timeout:", cfg.get("timeout_ms"))


if __name__ == "__main__":
    main()
EOF

# 10c. Markdown prose: word-level edits, a deleted bullet, and a whole inserted section.
cat <<'EOF' >"$L/text/release-notes.md"
# Release Notes

## Version 1.0
Initial release of the tool.
It supports local folder comparison.

## Known Issues
- Slow on very large trees.
- No dark mode yet.
EOF
cat <<'EOF' >"$R/text/release-notes.md"
# Release Notes

## Version 1.1
Second release of the tool.
It supports local folder comparison and side-by-side merge.

## New in this version
- Side-by-side file view.
- Copy changes between the two sides.

## Known Issues
- Slow on very large trees.
EOF

# 10d. CSV: a changed cell (word-level), a deleted row, an added row.
cat <<'EOF' >"$L/text/data.csv"
id,name,role
1,Alice,admin
2,Bob,editor
3,Carol,viewer
EOF
cat <<'EOF' >"$R/text/data.csv"
id,name,role
1,Alice,admin
2,Bob,owner
4,Dave,viewer
EOF

# 10e. Whitespace-only differences (4-space vs tab indent + a trailing space) — same logical
#      content. Demo data for a future "ignore whitespace" compare option.
printf 'function greet(name) {\n    return "hi " + name;\n}\n' >"$L/text/whitespace.js"
printf 'function greet(name) {\n\treturn "hi " + name; \n}\n'  >"$R/text/whitespace.js"

# 10f. One very long line that changes only near its end — exercises horizontal handling/wrapping.
{ printf 'const banner = "'; for i in $(seq 1 40); do printf 'lorem ipsum %s ' "$i"; done; printf 'END-LEFT";\n'; }  >"$L/text/longline.js"
{ printf 'const banner = "'; for i in $(seq 1 40); do printf 'lorem ipsum %s ' "$i"; done; printf 'END-RIGHT";\n'; } >"$R/text/longline.js"

# 10g. Wide CSV: many long rows, with a few small per-cell changes buried far into the line — to
#      exercise the line-detail pane's horizontal scroll + word-change stepper.
awk 'BEGIN { cols = 25; rows = 12;
  for (r = 1; r <= rows; r++) { line = "row" r; for (c = 1; c <= cols; c++) line = line ",val_" r "_" c; print line } }' >"$L/text/wide.csv"
awk 'BEGIN { cols = 25; rows = 12;
  for (r = 1; r <= rows; r++) { line = "row" r;
    for (c = 1; c <= cols; c++) { v = "val_" r "_" c; if ((r==3 && c==20) || (r==7 && c==23) || (r==10 && c==8)) v = v "_CHANGED"; line = line "," v }
    print line } }' >"$R/text/wide.csv"

# 10h. JSON with VERY long lines (URLs) and a multi-line block change — exercises the detail pane's
#      gap-aligned wide-line rendering + horizontal scroll on a real-looking config.
seg=$(printf 'path-segment-%s/' $(seq 1 30))
urla_l="https://api.example.com/v1/${seg}alpha?token=AAAAAAAAAAAA&region=eu-west-1&trace=on"
urla_r="https://api.example.com/v2/${seg}alpha?token=BBBBBBBBBBBB&region=us-east-1&trace=off"
urlb="https://cdn.example.com/v1/${seg}beta?token=CCCCCCCCCCCC&region=eu-west-1&trace=on"
urlc="https://backup.example.com/v1/${seg}gamma?token=DDDDDDDDDDDD&region=ap-south-1&trace=on"
cat <<EOF >"$L/text/urls.json"
{
  "name": "confold",
  "version": "0.3.0",
  "endpoints": [
    "$urla_l",
    "$urlb"
  ],
  "retries": 3
}
EOF
cat <<EOF >"$R/text/urls.json"
{
  "name": "confold",
  "version": "0.4.0",
  "endpoints": [
    "$urla_r",
    "$urlb",
    "$urlc"
  ],
  "timeout_ms": 5000
}
EOF

echo "  + text/ : config.json, app.py, release-notes.md, data.csv, whitespace.js, longline.js, wide.csv, urls.json (rich diffs)"

# 10i. Image pairs (24-bit BMP, no deps) for the image comparator. Variants exercise every mode:
#      base = colourful gradient; multi = 3 separate coloured blocks (→ region nav); subtle = faint +18
#      tint in a band (→ tolerance slider).
mkdir -p "$L/img" "$R/img"
gen_bmp() { # <outfile> <W> <H> <variant: base|multi|subtle>
  awk -v W="$2" -v H="$3" -v variant="$4" '
    function le4(n,  i) { for (i = 0; i < 4; i++) { printf "%c", n % 256; n = int(n / 256) } }
    function le2(n,  i) { for (i = 0; i < 2; i++) { printf "%c", n % 256; n = int(n / 256) } }
    function clamp(v) { return v < 0 ? 0 : (v > 255 ? 255 : int(v)) }
    BEGIN {
      rowsize = W * 3; pad = (4 - (rowsize % 4)) % 4; imgsize = (rowsize + pad) * H;
      filesize = 54 + imgsize;
      printf "BM"; le4(filesize); le2(0); le2(0); le4(54);
      le4(40); le4(W); le4(H); le2(1); le2(24); le4(0); le4(imgsize); le4(0); le4(0); le4(0); le4(0);
      for (y = 0; y < H; y++) {
        for (x = 0; x < W; x++) {
          r = x * 255 / W; g = y * 255 / H; b = (x + y) * 255 / (W + H);
          if (int((x + y) / 8) % 2 == 0) r = r * 0.65; # diagonal stripes for visual texture
          if (variant == "multi") {
            # three well-separated blocks (gaps > the 16px clustering cell → three distinct regions)
            if (x >= 8 && x < 26 && y >= 8 && y < 26) { r = 230; g = 30; b = 30 }
            else if (x >= 58 && x < 80 && y >= 8 && y < 26) { r = 30; g = 200; b = 60 }
            else if (x >= 32 && x < 54 && y >= 60 && y < 82) { r = 40; g = 70; b = 230 }
          } else if (variant == "subtle") {
            if (x >= 30 && x < 66 && y >= 30 && y < 66) { r += 18; g += 18; b += 18 }
          }
          printf "%c%c%c", clamp(b), clamp(g), clamp(r);
        }
        for (k = 0; k < pad; k++) printf "%c", 0;
      }
    }' >"$1"
}
gen_bmp "$L/img/photo.bmp"   96 96 base;  gen_bmp "$R/img/photo.bmp"   96 96 multi   # 3 diff regions → nav
gen_bmp "$L/img/subtle.bmp"  96 96 base;  gen_bmp "$R/img/subtle.bmp"  96 96 subtle  # faint → tolerance
gen_bmp "$L/img/resized.bmp" 96 96 base;  gen_bmp "$R/img/resized.bmp" 120 80 base   # different dimensions
gen_bmp "$L/img/same.bmp"    96 96 base;  gen_bmp "$R/img/same.bmp"    96 96 base    # identical (0%)
echo "  + img/ : photo.bmp (3 regions), subtle.bmp (tolerance), resized.bmp (dims), same.bmp (identical)"

# 10g. Optional LARGE text file pair (text/large.txt) — same content with a change every 500 lines.
#      Shows as "different"; double-click it to exercise the side-by-side virtualization. `awk` keeps it
#      fast even for hundreds of thousands of lines.
if [ "$biglines" -gt 0 ]; then
  awk -v n="$biglines" 'BEGIN { for (i = 1; i <= n; i++) print "line " i ": the quick brown fox jumps over the lazy dog" }' >"$L/text/large.txt"
  awk -v n="$biglines" 'BEGIN { for (i = 1; i <= n; i++) print "line " i ": the quick brown fox " (i % 500 == 0 ? "JUMPS over the lazy cat" : "jumps over the lazy dog") }' >"$R/text/large.txt"
  echo "  + text/large.txt : $biglines lines/side (a change every 500 lines)"
  if [ "$biglines" -gt 34000 ]; then
    echo "    NOTE: > ~2 MB — triggers the large-file warning dialog; opens in hunks-only read-only mode"
  fi
fi

# 10h. Large file pairs for the hunks-only diff feature (> 2 MB TEXT_CAP).
#      These are always generated (no extra argument needed).
#
#      data/server.log — 50 000-line simulated server log (~2.5 MB/side).
#        15 differences spread every ~3 300 lines → well under the 100-hunk default → is_complete=true.
#        Tests: warning dialog, hunks view, "N of N hunks · full file" status.
#
#      data/noisy.log  — 50 000-line log with a change every 200 lines (250 differences).
#        250 hunks >> 100-hunk default → is_complete=false → "Load more" button appears.
#        Tests: partial view, pagination.
mkdir -p "$L/data" "$R/data"

awk 'BEGIN {
  for (i = 1; i <= 50000; i++) {
    h = int(i/3600) % 24; m = int(i/60) % 60; s = i % 60;
    ts = sprintf("2024-06-17 %02d:%02d:%02d", h, m, s);
    if (i % 3334 == 0)
      printf "[%s] ERROR  Connection refused on port 5432 (attempt %d)\n", ts, int(i/3334)
    else
      printf "[%s] INFO   Request %d processed in %dms status=200\n", ts, i, (i*17)%200+5
  }
}' >"$L/data/server.log"

awk 'BEGIN {
  for (i = 1; i <= 50000; i++) {
    h = int(i/3600) % 24; m = int(i/60) % 60; s = i % 60;
    ts = sprintf("2024-06-17 %02d:%02d:%02d", h, m, s);
    if (i % 3334 == 0)
      printf "[%s] ERROR  Retrying after 5s — replica db-secondary unreachable (attempt %d)\n", ts, int(i/3334)
    else
      printf "[%s] INFO   Request %d processed in %dms status=200\n", ts, i, (i*17)%200+5
  }
}' >"$R/data/server.log"

awk 'BEGIN {
  for (i = 1; i <= 50000; i++) {
    h = int(i/3600) % 24; m = int(i/60) % 60; s = i % 60;
    ts = sprintf("2024-06-17 %02d:%02d:%02d", h, m, s);
    if (i % 200 == 0)
      printf "[%s] WARN   cache-miss ratio=%.2f%% threshold exceeded\n", ts, 45 + (i % 30)
    else
      printf "[%s] DEBUG  cache-hit key=%08x ratio=%.2f%%\n", ts, i*2654435761, 95 - (i % 10) * 0.5
  }
}' >"$L/data/noisy.log"

awk 'BEGIN {
  for (i = 1; i <= 50000; i++) {
    h = int(i/3600) % 24; m = int(i/60) % 60; s = i % 60;
    ts = sprintf("2024-06-17 %02d:%02d:%02d", h, m, s);
    if (i % 200 == 0)
      printf "[%s] WARN   cache-miss ratio=%.2f%% — eviction triggered, freed %dMB\n", ts, 45 + (i % 30), 64 + (i % 256)
    else
      printf "[%s] DEBUG  cache-hit key=%08x ratio=%.2f%%\n", ts, i*2654435761, 95 - (i % 10) * 0.5
  }
}' >"$R/data/noisy.log"

sz_srv=$(wc -c <"$L/data/server.log"); sz_nsy=$(wc -c <"$L/data/noisy.log")
echo "  + data/server.log : 50 000 lines, 15 differences (~${sz_srv} bytes) — large-file hunks demo (complete)"
echo "  + data/noisy.log  : 50 000 lines, 250 differences (~${sz_nsy} bytes) — large-file hunks demo (load-more)"

# 11. Optional bulk files to stress the tree / virtualization. Mostly identical, with variety:
#     every 100th differs, every 250th is left-only.
if [ "$count" -gt 0 ]; then
  mkdir -p "$L/bulk" "$R/bulk"
  for i in $(seq 1 "$count"); do
    printf 'bulk %s\n' "$i" >"$L/bulk/f$i.txt"
    if [ $((i % 250)) -eq 0 ]; then
      :                                                    # left-only (no right file)
    elif [ $((i % 100)) -eq 0 ]; then
      printf 'bulk %s CHANGED\n' "$i" >"$R/bulk/f$i.txt"   # different
    else
      printf 'bulk %s\n' "$i" >"$R/bulk/f$i.txt"           # identical
    fi
  done
  echo "  + $count bulk files/side under bulk/ (every 100th differs, every 250th left-only)"
fi

echo "demo trees created:"
echo "  left : $L"
echo "  right: $R"
