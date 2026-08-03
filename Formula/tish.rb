# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.2.2"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.2.2/tish-darwin-arm64"
      sha256 "0f7d287dd297fcf8ce9a11451a3f8c0a3f18c65aa6c3707e531783769840d0b2"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.2.2/tish-darwin-x64"
      sha256 "33737a41697aca659fe6cf243d1ba0ce7f8f42d25ac971a928a6920af2ccf683"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.2.2/tish-linux-arm64"
      sha256 "19fec27e29e52ff539029a9c694255c7f73ffd71a17f26b00e691b9da865e4ef"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.2.2/tish-linux-x64"
      sha256 "6717d9546706f9af222419d39025e75efb105d56f5f7a1a7f6d29f53e6812279"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
