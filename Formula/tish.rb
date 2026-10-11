# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "4.1.4"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.1.4/tish-darwin-arm64"
      sha256 "7624dd46148e06d2b7a9d388e0d854c80d7719c63c90175cba3f5f1933982a3b"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.1.4/tish-darwin-x64"
      sha256 "0daf3b556735441c6680c875cd06b0e457fc5ad99ce677b04e87c46fe68a9786"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.1.4/tish-linux-arm64"
      sha256 "d5f325ceca7dc80a698db7d86c485cce93289a058ddb558a0185b67a3b4f0dfd"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.1.4/tish-linux-x64"
      sha256 "9cdd96bb2d6cec70505f83dba4840bdf4cbbbd799436132fabf5d61671c5722a"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
