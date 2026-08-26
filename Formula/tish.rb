# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.9.2"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.9.2/tish-darwin-arm64"
      sha256 "7c69d90960b121cd87e959d529a2ee4ae1852a9bc3e40de4cc7cfba2d0376732"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.9.2/tish-darwin-x64"
      sha256 "0ab015d018967fc0658baaa2dff67a73c47610d2f4793ed902bafce68bde435f"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.9.2/tish-linux-arm64"
      sha256 "5a087d6d8efbf4aade3cba43bb6f4b1ec9f7601f44a0d834b36041e9618a4de2"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.9.2/tish-linux-x64"
      sha256 "102e52990d97540a7885b94d3fc7501bf0c3cda585f2db05fb26243076447e6d"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
