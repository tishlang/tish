# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.2.7"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.7/tish-darwin-arm64"
      sha256 "472bed7c1a9986edb1fa3d100c026641a5982b7028c556630b3b7950ea1eb323"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.7/tish-darwin-x64"
      sha256 "78015ba2f6bcbb9e6395c5a80e55851819c9324dd9fcfe95a5bcede0bde8dc21"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.7/tish-linux-arm64"
      sha256 "e57711adf511707fc30d119082a2df01ae088d5f6d7bd01eb9902b669d27963b"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.7/tish-linux-x64"
      sha256 "d19bdf4b82902b8ab336cd08358d3f4c31f276948bc04b9b4ed22f67740ceb67"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
