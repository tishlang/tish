# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.35.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.35.0/tish-darwin-arm64"
      sha256 "4323e14c5489548948719097cc2f092790a352b135a44eebbbde1e381ac4125b"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.35.0/tish-darwin-x64"
      sha256 "5aac29bbe8931772eb25d896460748c5406e23b9ffe9d9fc247e6e09bfbb5c2e"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.35.0/tish-linux-arm64"
      sha256 "f06f16876a66366bd3f894ad44c5819ddc312ca41c8da839d07b3f9c29f90f0e"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.35.0/tish-linux-x64"
      sha256 "dc1d9d2f7b90382e94702e586f4166d37d3380f3ab76f9d68ab84821478b1e36"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
