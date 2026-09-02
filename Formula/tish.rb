# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.10.11"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.11/tish-darwin-arm64"
      sha256 "c72699b16a0e2169e1c833c1415cd20dcf48b1fcaed917162844fd8b23003f84"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.11/tish-darwin-x64"
      sha256 "bdf093208c7633f89d0ebe346ab9b8887bf8f4def0296b8d584fb55da5b3b3ad"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.11/tish-linux-arm64"
      sha256 "686e913a287eddc5347705ad1a719a8945e109f89dabb5f7d9472f2aea3ea1df"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.11/tish-linux-x64"
      sha256 "8e868687c967695fed5e56a67c0b232a3ffc1f09f16169748ed486f883eb9fda"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
