class Confold < Formula
  desc "Fast, keyboard-driven folder and file comparison, migrate and sync tool"
  homepage "https://confold.com"
  url "https://github.com/confold/confold/releases/download/v0.5.1/Confold_0.5.1_amd64.AppImage"
  sha256 "cc08bf6873328f5d33320a7fb68c2b8211021227e412a0d2ad65b081d7a40a50"
  version "0.5.1"
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
