# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.1.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.1.0/tish-darwin-arm64"
      sha256 "e2f430bf0590133304d7e3b9a0befd074d37282abf403b81e375861e3f1a2852"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.1.0/tish-darwin-x64"
      sha256 "a4b6461c6a1599c0e724cce4f00f6406ad9395430125607980879ef4d3541164"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.1.0/tish-linux-arm64"
      sha256 "3da7b0688fb1eac91513935e10c553b76839e79ee05080bbc4c5d61f63594118"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.1.0/tish-linux-x64"
      sha256 "dc22178611eed00c0b592ae1a32e8140d0cb8d9172a70e3d6b3f8bbd85c3af2d"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
