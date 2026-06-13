# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.2.5"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.5/tish-darwin-arm64"
      sha256 "d8951d3d261495015764f0fa668bfa6c75314d1bed7e3fafa9068c7c255c6c22"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.5/tish-darwin-x64"
      sha256 "a5dc5458857859b251a038b06b2e696e741b24d016fab545877df53e1a5fa510"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.5/tish-linux-arm64"
      sha256 "5e9a9378e31f1887ecbbed98d20ec21f09e33ff3818070c3fe30c7950a1a947f"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.5/tish-linux-x64"
      sha256 "7b61ef31b5b10faced29c92c0ba1a9e6776946d20776bac5b6492925408c02c2"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
