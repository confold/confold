# Local Development

How to set up a dev environment, run the app, and generate test data.

## Prerequisites

| Tool | Version | Install |
|---|---|---|
| **Rust** | 1.96.0 (pinned via `rust-toolchain.toml`) | [rustup.rs](https://rustup.rs) |
| **Node.js** | 20+ | [nodejs.org](https://nodejs.org) or `nvm install 20` |
| **pnpm** | 9+ | `npm install -g pnpm` or `brew install pnpm` |
| **Docker** | any (only for SFTP demo server) | [Docker Desktop](https://docker.com) |

**macOS system deps:** Xcode Command Line Tools (`xcode-select --install`).

**Linux system deps (Tauri webview):**

```sh
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev librsvg2-dev build-essential
```

**Windows:** WebView2 (preinstalled on Windows 10/11).

## Environment setup

### 1. Rust

Install via [rustup.rs](https://rustup.rs). The exact version (1.96.0) is pinned in
`rust-toolchain.toml` — `rustup` auto-installs it on first `cargo` invocation in this repo.

### 2. direnv (loads Rust PATH per-project)

The repo includes a `.envrc` that loads `~/.cargo/bin` into PATH **only when you're inside the
project directory** — no global `.zshrc` pollution. Requires [direnv](https://direnv.net):

```sh
brew install direnv                   # if not already installed
eval "$(direnv hook zsh)"             # add to ~/.zshrc.personal.after (once)
direnv allow                          # authorize the .envrc (once per clone)
```

After that, `cargo` is available automatically whenever you `cd` into the repo. If you see
`direnv: error /path/.envrc is blocked`, run `direnv allow` again.

> **Alternative (no direnv):** add `. "$HOME/.cargo/env"` to your `~/.zshrc` to load cargo
> globally. Or run `source ~/.cargo/env` manually before `pnpm tauri dev`.

### 2. Frontend dependencies

```sh
cd confold-app
pnpm install
```

### 3. Verify

```sh
cargo test --all          # engine + CLI tests
cd confold-app && pnpm test && pnpm check   # frontend tests + type check
```

## Running the desktop app

```sh
cd confold-app
pnpm tauri dev
```

Compiles the Rust backend + launches the SvelteKit frontend with hot-reload in a native
window. First build takes ~4 min; subsequent launches are near-instant.

> `pnpm dev` runs only the Vite dev server (frontend only, no Tauri commands). Useful for
> rapid UI prototyping but the app won't be able to compare/migrate/sync.

## Generating test data

### Local folder pairs — `scripts/gen-demo.sh`

Creates two folder trees (`origin/` and `destination/`) with a rich mix of differences: identical,
modified, unique, binary, image (BMP), multi-hunk text (JSON, Python, Markdown, CSV, JS),
large files (50K-line logs), and optional bulk stress files.

```sh
# Basic demo tree (no args beyond target dir)
scripts/gen-demo.sh /tmp/confold-demo

# With 500 bulk files + a 100K-line large text file
scripts/gen-demo.sh /tmp/confold-demo 500 100000
```

Then in the app, set Origen = `/tmp/confold-demo/origin`, Destino = `/tmp/confold-demo/destination`,
method = `full`.

**What's in the tree:**

| Path | Exercises |
|---|---|
| `same.txt` | identical detection |
| `samesize.dat` | same-size different-bytes (size method says equal, full says different) |
| `readme.md` | different-size text diff |
| `only_origin.txt` / `only_dest.txt` | unique files |
| `only_origin_tree/` / `only_dest_tree/` | unique subtrees |
| `image.bin`, `blob.bin` | binary compare (identical + different) |
| `big.bin` | 8 MiB file (above quick-threshold sampling) |
| `cache.tmp` | glob exclude filters |
| `text/config.json` | word-level JSON diff |
| `text/app.py` | multi-hunk Python (added/removed functions) |
| `text/release-notes.md` | prose with inserted/deleted sections |
| `text/data.csv` | cell-level CSV diff |
| `text/whitespace.js` | whitespace-only difference |
| `text/longline.js` | very long line, change near the end |
| `text/wide.csv` | 25-column CSV, small per-cell changes |
| `text/urls.json` | very long URLs with embedded changes |
| `img/photo.bmp` | 3 separated diff regions (region navigation) |
| `img/subtle.bmp` | faint tint (tolerance slider) |
| `img/resized.bmp` | different dimensions |
| `img/same.bmp` | identical image (0% diff) |
| `data/server.log` | 50K lines, 15 hunks (large-file complete view) |
| `data/noisy.log` | 50K lines, 250 hunks (large-file pagination) |

### SFTP server — `scripts/sftp-demo.sh`

Runs the `atmoz/sftp` Docker image. **Requires Docker.**

```sh
scripts/sftp-demo.sh                    # serves /tmp/ftp on port 2222
scripts/sftp-demo.sh /path/to/dir 2222  # custom dir + port
```

In the app, paste this URL into a source picker:

```
sftp://ftp@localhost:2222/data
```

Password: `ftp`. The script auto-seeds `readme.txt`, `config.json`, and `sub/note.txt` on
first run. Ctrl-C stops the server (container is `--rm`, auto-cleanup).

### S3 server — `scripts/s3-demo.sh`

Runs a **pure-Rust** S3-compatible server (`s3s` + `s3s-fs`). **No Docker needed.**

```sh
scripts/s3-demo.sh                      # serves /tmp/confold-s3 on port 4566
scripts/s3-demo.sh /path/to/dir 4566    # custom dir + port
```

In the app, paste this URL into a source picker:

```
s3://confold:confold-secret@127.0.0.1:4566/data
```

Or fill the fields manually:

| Field | Value |
|---|---|
| Endpoint | `http://127.0.0.1:4566` |
| Region | `us-east-1` |
| Bucket | `data` |
| Access key | `confold` |
| Secret key | `confold-secret` |

Auto-seeds `readme.txt` and `sub/note.txt` in the `data` bucket on first run. Ctrl-C to stop.

## CLI (headless)

```sh
cargo build --release
./target/release/confold compare <origin> <destination> --method full
```

Markers: `=` identical | `~` different | `<` origin-only | `>` dest-only | `.` skipped | `!` error

Useful flags: `--method full|quick|size|mtime|size-mtime`, `--include/--exclude <glob>`,
`--format text|json`, `--no-recursive`, `--fail-on-diff`.

## Quality gates

Run these before submitting changes:

```sh
# Engine + CLI
cargo test --all
cargo lint          # clippy with -D warnings (alias in .cargo/config.toml)
cargo fmt --all --check

# Frontend
cd confold-app && pnpm test && pnpm check
```

## Troubleshooting

### `cargo metadata: No such file or directory (os error 2)`

`~/.cargo/bin` is not in the PATH that `pnpm` child processes see. The repo's `.envrc` handles
this via direnv — make sure direnv is installed, hooked into your shell, and you've run
`direnv allow` in the repo root. (You should see a `direnv: loading .envrc` message on `cd`.)

### `rustup` says the toolchain version doesn't match

The repo pins Rust 1.96.0 in `rust-toolchain.toml`. `rustup` auto-installs it on first `cargo`
invocation. If it didn't, run `rustup install 1.96.0` manually.

### `pnpm tauri dev` hangs or shows a blank window

On first build, the Rust backend takes several minutes to compile. Watch the terminal for
`Running `target/debug/confold-app`` — that means the backend is ready and the window should
appear.

### Port already in use (2222 / 4566)

Pass a different port as the second argument: `scripts/sftp-demo.sh /tmp/ftp 2223` or
`scripts/s3-demo.sh /tmp/s3 4567`.
