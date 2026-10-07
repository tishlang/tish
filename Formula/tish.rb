# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "4.0.0"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.0.0/tish-darwin-arm64"
      sha256 "9eba672e15ab63cf5c017b32cec4a94c7332a3fee649f69d169a2bb4d66e4eee"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.0.0/tish-darwin-x64"
      sha256 "00b426c22c028ee35157a678c5d53c04fa88a1302c0fa969c7f02c8f2837bfe1"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.0.0/tish-linux-arm64"
      sha256 "f09d2a64e0eb5f32d4be87d7097bcd0afb3870f12df0a7095c56bc221cd72378"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.0.0/tish-linux-x64"
      sha256 "3dc431e45925a6c4fd1af8bfff017a6679661cfb7caf4a9a6eaeee1ab8b9aaf0"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
