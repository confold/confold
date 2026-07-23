cask "confold" do
  # Update version + both sha256 values on each release (use scripts/bump-packaging.sh).
  version "0.6.1"

  on_arm do
    sha256 "28938f02d937850959c2ec0660737c9c71e652c44d93128bec7b264118f1a724"
    url "https://github.com/confold/confold/releases/download/v#{version}/Confold_#{version}_aarch64.dmg"
  end

  on_intel do
    sha256 "e3ea75f75e465ea34d89eb524124e5bdfe43c0fd8615de1e070c35d1e3149968"
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
