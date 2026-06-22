cask "confold" do
  # Update version + both sha256 values on each release (use scripts/bump-packaging.sh).
  version "0.5.1"

  on_arm do
    sha256 "47dc3f27bb4c999c4a7eb7f53a7516355029705667f6311c042913744801e7a5"
    url "https://github.com/confold/confold/releases/download/v#{version}/Confold_#{version}_aarch64.dmg"
  end

  on_intel do
    sha256 "f6a7baad1ded7ccaf3f875a6325def6e7e59410a68df894e8933b71f6fd88fe3"
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
