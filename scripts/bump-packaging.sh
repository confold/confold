#!/usr/bin/env bash
# Update all packaging manifests to a new version, reading the release digests from GitHub.
# Usage: ./scripts/bump-packaging.sh <version>    (e.g. 0.5.1 or v0.5.1)
# Requires: gh (GitHub CLI, authenticated), perl, awk.
#
# Confold ships Tauri installers (no portable archive), so the channels map like this:
#   Homebrew cask      → macOS .dmg   (arm64 + x64)
#   Homebrew formula   → Linux .AppImage
#   Scoop              → Windows NSIS -setup.exe (extracted as 7z)
#   winget             → Windows .msi (WiX)
#   Chocolatey         → Windows NSIS -setup.exe (silent install)
set -euo pipefail

VERSION="${1:-}"
[[ -z "$VERSION" ]] && { echo "Usage: $0 <version>  (e.g. 0.5.1)" >&2; exit 1; }
VERSION="${VERSION#v}"
TAG="v${VERSION}"
REPO="confold/confold"

PREV_VERSION=$(awk '/^[[:space:]]*version / {gsub(/"/, "", $2); print $2; exit}' dist/brew/Casks/confold.rb)
[[ -z "$PREV_VERSION" ]] && { echo "Could not read current version from dist/brew/Casks/confold.rb" >&2; exit 1; }

echo "Bumping ${PREV_VERSION} → ${VERSION}"

# ── Fetch digests from the GitHub release ─────────────────────────────────────
echo "Fetching release digests for ${TAG} from ${REPO}..."
ASSETS=""
for attempt in 1 2 3; do
  ASSETS=$(gh release view "$TAG" --repo "$REPO" --json assets \
    --jq '.assets[] | "\(.name) \(.digest)"' 2>/dev/null || true)
  if [[ -n "$ASSETS" ]]; then
    break
  fi
  echo "Attempt $attempt: release assets not yet visible, waiting 10s..."
  sleep 10
done

if [[ -z "$ASSETS" ]]; then
  echo "Error: could not fetch release assets after 3 attempts" >&2
  exit 1
fi

extract() { echo "$ASSETS" | awk "/$1/ {print \$2}" | sed 's/^sha256://'; }

SHA_MACOS_ARM=$(extract "Confold_${VERSION}_aarch64\\.dmg")
SHA_MACOS_X86=$(extract "Confold_${VERSION}_x64\\.dmg")
SHA_LINUX=$(extract     "Confold_${VERSION}_amd64\\.AppImage")
SHA_WIN_NSIS=$(extract  "Confold_${VERSION}_x64-setup\\.exe")
SHA_WIN_MSI=$(extract   "Confold_${VERSION}_x64_en-US\\.msi")

# Report missing digests explicitly
MISSING=()
[[ -z "$SHA_MACOS_ARM" ]] && MISSING+=("macOS arm64 (.dmg)")
[[ -z "$SHA_MACOS_X86" ]] && MISSING+=("macOS x64 (.dmg)")
[[ -z "$SHA_LINUX" ]] && MISSING+=("Linux (.AppImage)")
[[ -z "$SHA_WIN_NSIS" ]] && MISSING+=("Windows NSIS (.exe)")
[[ -z "$SHA_WIN_MSI" ]] && MISSING+=("Windows MSI (.msi)")

if [[ ${#MISSING[@]} -gt 0 ]]; then
  echo "Error: missing required release assets:" >&2
  for m in "${MISSING[@]}"; do echo "  - $m" >&2; done
  echo "" >&2
  echo "Available assets:" >&2
  printf "%s\n" "$ASSETS" >&2
  exit 1
fi

SHA_WIN_NSIS_UP=$(echo "$SHA_WIN_NSIS" | tr '[:lower:]' '[:upper:]')
SHA_WIN_MSI_UP=$(echo  "$SHA_WIN_MSI"  | tr '[:lower:]' '[:upper:]')
printf "macOS arm64: %s\nmacOS x64:   %s\nLinux:       %s\nWin NSIS:    %s\nWin MSI:     %s\n\n" \
  "$SHA_MACOS_ARM" "$SHA_MACOS_X86" "$SHA_LINUX" "$SHA_WIN_NSIS" "$SHA_WIN_MSI"

# Helper: in-place perl substitution (portable across macOS/Linux).
p() { perl -i -pe "$1" "$2"; }

# ── Homebrew cask (macOS arm64 + x64 .dmg) ────────────────────────────────────
echo "Updating Homebrew cask..."
p "s/^  version \".*\"/  version \"${VERSION}\"/" dist/brew/Casks/confold.rb
awk -v arm="$SHA_MACOS_ARM" -v intel="$SHA_MACOS_X86" '
  /on_arm do/   { in_arm=1 }
  /on_intel do/ { in_intel=1 }
  /^  end/      { in_arm=0; in_intel=0 }
  in_arm   && /sha256/ { sub(/"[0-9a-f]{64}"/, "\"" arm   "\"") }
  in_intel && /sha256/ { sub(/"[0-9a-f]{64}"/, "\"" intel "\"") }
  { print }
' dist/brew/Casks/confold.rb > dist/brew/Casks/confold.rb.tmp \
  && mv dist/brew/Casks/confold.rb.tmp dist/brew/Casks/confold.rb

# ── Homebrew formula (Linux .AppImage) ────────────────────────────────────────
echo "Updating Homebrew formula..."
p "s|releases/download/v[0-9.]+/Confold_[0-9.]+_amd64\\.AppImage|releases/download/v${VERSION}/Confold_${VERSION}_amd64.AppImage|" dist/brew/Formula/confold.rb
p "s/^  sha256 \"[0-9a-f]{64}\"/  sha256 \"${SHA_LINUX}\"/"                                                                       dist/brew/Formula/confold.rb
p "s/^  version \"[0-9.]+\"/  version \"${VERSION}\"/"                                                                            dist/brew/Formula/confold.rb

# ── Scoop (Windows NSIS via 7z extraction) ────────────────────────────────────
echo "Updating Scoop..."
p "s/\"version\": \"[^\"]*\"/\"version\": \"${VERSION}\"/"   dist/scoop/bucket/confold.json
p "s/\"hash\": \"[^\"]*\"/\"hash\": \"${SHA_WIN_NSIS}\"/"    dist/scoop/bucket/confold.json
# Literal 64bit url: bump BOTH the /download/vX/ path and the Confold_X filename. The
# autoupdate url uses $version templating, so [0-9.]+ leaves it untouched.
p "s|download/v[0-9.]+/Confold_[0-9.]+_x64-setup\\.exe|download/v${VERSION}/Confold_${VERSION}_x64-setup.exe|" dist/scoop/bucket/confold.json

# ── winget — create the new version folder, bump MSI url + sha ────────────────
echo "Updating winget..."
WINGET_OLD="dist/winget/manifests/c/Confold/Confold/${PREV_VERSION}"
WINGET_NEW="dist/winget/manifests/c/Confold/Confold/${VERSION}"
[[ ! -d "$WINGET_NEW" ]] && cp -r "$WINGET_OLD" "$WINGET_NEW"
for f in "$WINGET_NEW"/*.yaml; do
  p "s/PackageVersion: .*/PackageVersion: ${VERSION}/"                                                "$f"
  p "s|/v[0-9.]+/Confold_[0-9.]+_x64_en-US\\.msi|/v${VERSION}/Confold_${VERSION}_x64_en-US.msi|g"     "$f"
  p "s/InstallerSha256: [0-9A-F]{64}/InstallerSha256: ${SHA_WIN_MSI_UP}/"                             "$f"
done

# ── Chocolatey (Windows NSIS, silent install) ─────────────────────────────────
echo "Updating Chocolatey..."
p "s|/v[0-9.]+/Confold_[0-9.]+_x64-setup\\.exe|/v${VERSION}/Confold_${VERSION}_x64-setup.exe|" dist/chocolatey/tools/chocolateyinstall.ps1
p "s/'[0-9A-F]{64}'/'${SHA_WIN_NSIS_UP}'/"                                                      dist/chocolatey/tools/chocolateyinstall.ps1
p "s|<version>[^<]*</version>|<version>${VERSION}</version>|"                                   dist/chocolatey/confold.nuspec
p "s|releases/tag/v[0-9.]+|releases/tag/v${VERSION}|g"                                          dist/chocolatey/confold.nuspec
p "s|jsdelivr.net/gh/confold/confold@v[0-9.]+/|jsdelivr.net/gh/confold/confold@v${VERSION}/|"   dist/chocolatey/confold.nuspec
p "s|releases/tag/v[0-9.]+|releases/tag/v${VERSION}|g"                                          dist/chocolatey/tools/VERIFICATION.txt
p "s|release view v[0-9.]+|release view v${VERSION}|g"                                          dist/chocolatey/tools/VERIFICATION.txt
p "s/Confold_[0-9.]+_x64-setup\\.exe/Confold_${VERSION}_x64-setup.exe/g"                        dist/chocolatey/tools/VERIFICATION.txt
p "s/[0-9A-F]{64}/${SHA_WIN_NSIS_UP}/"                                                          dist/chocolatey/tools/VERIFICATION.txt

# ── Website (versioned direct-download links + version label) ─────────────────
# Cloudflare Pages redeploys web/ on push to main, so the download buttons track each release.
echo "Updating website download links..."
p "s|releases/download/v[0-9.]+/Confold|releases/download/v${VERSION}/Confold|g"  web/index.html
p "s|Confold_[0-9.]+_aarch64\\.dmg|Confold_${VERSION}_aarch64.dmg|g"              web/index.html
p "s|Confold_[0-9.]+_x64\\.dmg|Confold_${VERSION}_x64.dmg|g"                      web/index.html
p "s|Confold_[0-9.]+_x64-setup\\.exe|Confold_${VERSION}_x64-setup.exe|g"          web/index.html
p "s|Confold_[0-9.]+_x64_en-US\\.msi|Confold_${VERSION}_x64_en-US.msi|g"          web/index.html
p "s|Confold_[0-9.]+_amd64\\.AppImage|Confold_${VERSION}_amd64.AppImage|g"        web/index.html
p "s|Confold_[0-9.]+_amd64\\.deb|Confold_${VERSION}_amd64.deb|g"                  web/index.html
p "s|Confold-[0-9.]+-1\\.x86_64\\.rpm|Confold-${VERSION}-1.x86_64.rpm|g"          web/index.html
p "s|<code>v[0-9.]+</code>|<code>v${VERSION}</code>|"                             web/index.html
p "s/v[0-9]+\\.[0-9]+\\.[0-9]+( &nbsp;·)/v${VERSION}\$1/"                         web/index.html

echo ""
echo "Done — all manifests at ${VERSION}."
echo "Review: git diff dist/"
echo ""
printf "Publish (automated by .github/workflows/publish-*.yml on release publish):\n"
printf "  brew:        dist/brew/{Casks,Formula}/confold.rb  →  github.com/confold/homebrew-confold\n"
printf "  scoop:       dist/scoop/bucket/confold.json         →  github.com/confold/scoop-confold  bucket/\n"
printf "  winget:      PR to microsoft/winget-pkgs with dist/winget/manifests/c/Confold/Confold/%s/\n" "$VERSION"
printf "  chocolatey:  cd dist/chocolatey && choco pack && choco push\n"
