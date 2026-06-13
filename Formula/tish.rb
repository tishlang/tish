# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.2.3"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.3/tish-darwin-arm64"
      sha256 "ab517350a4c517cc819b2d73e854f844579778c5055465550183854ea92b9d72"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.3/tish-darwin-x64"
      sha256 "8c9d3aed99a9688ac6d79a2bcb63a4cdf1ccb7950e20e4918d3f4f7f12ec4607"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.3/tish-linux-arm64"
      sha256 "f1adb48f697bc83d69d10f00d884c688353abde03bc9fbfec0ffedad58f69a1f"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.3/tish-linux-x64"
      sha256 "03d14add1b80dc718dab18f9a07c1a4b9648f7d8e2a6456a58f1952707bbcc7f"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
