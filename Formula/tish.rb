# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.9.0"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.9.0/tish-darwin-arm64"
      sha256 "6c8905a4bc7bbfb5aeaf26e3d93212139eccd44fa9dde0af5be4c4eaab948f96"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.9.0/tish-darwin-x64"
      sha256 "f44e6cb9cebcf6e77eeb4abc3046d28910c97efc77f5a8d59794aa89f77f5ada"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.9.0/tish-linux-arm64"
      sha256 "83bab601d13b2f8109155c05749c742ce76a79db441ba4292ad7006bd01febfb"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.9.0/tish-linux-x64"
      sha256 "6ca277c5f9e03b449bfef9be218c86c6e6dcd2ab7a88e321b29398eb2c06d8b6"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
