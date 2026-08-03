# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.2.1"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.2.1/tish-darwin-arm64"
      sha256 "82ba08b4a3fe7bd1a38a2d5d11f8cd9dcb532b68490b5a233b0ef78b74de1383"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.2.1/tish-darwin-x64"
      sha256 "1d761176a665ddb6b3f71a44db68fab79957039d2a38f75265a06d65b3123598"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.2.1/tish-linux-arm64"
      sha256 "5a3644666050aa641a79f47f4c958698f18481d3608e4c5f5f65ff26f7900a37"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.2.1/tish-linux-x64"
      sha256 "ae2bd8f0964b6a9220c14584c5fb5f4f1bf8f62951208dcf8aea2c69029f10cd"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
