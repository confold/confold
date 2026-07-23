class Confold < Formula
  desc "Fast, keyboard-driven folder and file comparison, migrate and sync tool"
  homepage "https://confold.com"
  url "https://github.com/confold/confold/releases/download/v0.6.1/Confold_0.6.1_amd64.AppImage"
  sha256 "f4601d08c83f94c765d7f1b7d998b9cc87d232200cf87d25009c28b58e9e052e"
  version "0.6.1"
  license "Apache-2.0"

  # Confold is a Tauri GUI app — on Linux it ships as an AppImage, not a portable CLI binary.
  # Install the AppImage into libexec and expose a `confold` launcher that runs it in
  # extract-and-run mode, so it works on Linuxbrew hosts without a system FUSE.
  def install
    libexec.install "Confold_#{version}_amd64.AppImage" => "confold.AppImage"
    chmod 0755, libexec/"confold.AppImage"
    (bin/"confold").write <<~EOS
      #!/bin/bash
      export APPIMAGE_EXTRACT_AND_RUN=1
      exec "#{libexec}/confold.AppImage" "$@"
    EOS
    chmod 0755, bin/"confold"
  end

  test do
    assert_path_exists libexec/"confold.AppImage"
  end
end
