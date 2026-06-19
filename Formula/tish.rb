# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.10.1"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.10.1/tish-darwin-arm64"
      sha256 "e8d7818133b2eced5b56a48a1b5768dc36722947a2337f73475857034c1bb42c"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.10.1/tish-darwin-x64"
      sha256 "511adf3a150b880e41fed96284055e2539342be250d8ac7e47d2c81af7ca9403"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.10.1/tish-linux-arm64"
      sha256 "58ca8ebb3dc7e581747343d659b7735e43ad78719de8909f6ef357b1e4e02fef"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.10.1/tish-linux-x64"
      sha256 "e873decbabe5cb2d71ab776050e14b1488f40c0e350ace3bb4066894e9622ab0"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
