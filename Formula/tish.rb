# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.3.7"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.3.7/tish-darwin-arm64"
      sha256 "b886e98c4a8f925098f0f610609666ddbc01f297f1193fd6ee6ccd8096aec013"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.3.7/tish-darwin-x64"
      sha256 "3e8ec493bc8245e46570456c426771485cab2dd59c7aacb41d6b448a717f3287"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.3.7/tish-linux-arm64"
      sha256 "6015155aa4cb247ede4e1f5878bf71e34f88e86ce8af7a52c41a6a62ff6f8498"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.3.7/tish-linux-x64"
      sha256 "1592468a3fb02b8a0a2d6a1baa4c32d0ab75373c545daa2a3c04e0c49f2fdfe3"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
