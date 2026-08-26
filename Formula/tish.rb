# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.10.2"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.2/tish-darwin-arm64"
      sha256 "d8c4a27daae4efa2581b2c5808036b1b4b4e48034ed803e98bac543a5da88f8e"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.2/tish-darwin-x64"
      sha256 "4fe49953b1bbfeab7133923c50a89e38d2a25335e3b325eea16d9bae00ae81e7"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.2/tish-linux-arm64"
      sha256 "9e10ebaf88f5c55367b8361e2286d80917caa563850cdc881e2af98f6814dfb8"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.2/tish-linux-x64"
      sha256 "897bcc809711db212e876a669733d794f71641d8e2dfbd87dad1d7b8f7aaa7c0"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
