# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.0.3"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.0.3/tish-darwin-arm64"
      sha256 "38b8ae7c78243b56bb305f42a6f5cbaea27d0723c849c98ca15ca88d1b86bb2c"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.0.3/tish-darwin-x64"
      sha256 "819f2b6fd500282b5053b4668f463fbc3f2d7594930acf4cca1d8bd8a29b6555"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.0.3/tish-linux-arm64"
      sha256 "cfd403c57467458a95ab472dc3c5a3977cf3e19bb9c2766d020428c93cec4cd3"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.0.3/tish-linux-x64"
      sha256 "c65b8a0d6fbedca885ed1c1a65373e31654fe67527cecb4e10e50aed96a2130d"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
