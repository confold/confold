cask "confold" do
  # Update version + both sha256 values on each release (use scripts/bump-packaging.sh).
  version "0.6.0"

  on_arm do
    sha256 "60a9f7916e2ea86393523f348c7cbf6114dcfb7695507fb86687495468b8708e"
    url "https://github.com/confold/confold/releases/download/v#{version}/Confold_#{version}_aarch64.dmg"
  end

  on_intel do
    sha256 "bf627b1cef29fc66be52713394bbeabe75293cc392aec18ea15cad0d4103be7e"
    url "https://github.com/confold/confold/releases/download/v#{version}/Confold_#{version}_x64.dmg"
  end

  name "Confold"
  desc "Fast, keyboard-driven folder and file comparison, migrate and sync tool"
  homepage "https://confold.com"

  livecheck do
    url :url
    strategy :github_latest
  end

  app "Confold.app"

  uninstall quit: "com.confold.app"

  zap trash: [
    "~/Library/Application Support/com.confold.app",
    "~/Library/Caches/com.confold.app",
    "~/Library/Preferences/com.confold.app.plist",
    "~/Library/Saved Application State/com.confold.app.savedState",
    "~/Library/WebKit/com.confold.app",
  ]
end
