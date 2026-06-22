cask "confold" do
  # Update version + both sha256 values on each release (use scripts/bump-packaging.sh).
  version "0.5.0"

  on_arm do
    sha256 "9bc994e16528c38a30bc64e0fd12524e3ce45a14b9e1036172932c83959a8fa8"
    url "https://github.com/confold/confold/releases/download/v#{version}/Confold_#{version}_aarch64.dmg"
  end

  on_intel do
    sha256 "15e5e0a1108f8dfe000f03c82fed5edfa93fc42dea8834a7efa0e1940f7c5e0a"
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
