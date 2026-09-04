# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.12.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.12.0/tish-darwin-arm64"
      sha256 "9fb081a48e307bd30aab789c85719af3d10b958e4f43b522a2dfd60637a05d9f"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.12.0/tish-darwin-x64"
      sha256 "ab010144aa8deb10f2c958d2bae688cd4d6e59068c0b4a018bff7e9cb236c1f0"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.12.0/tish-linux-arm64"
      sha256 "5e5d9e69649db0a2c9042c6ee073933a1b2eaf7bcf0c101a532de5e199ff0ebc"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.12.0/tish-linux-x64"
      sha256 "3eb8f160444fcdff4251c21955c802fa1cc8b3f0451ebcf6c2b40e4a5495df84"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
