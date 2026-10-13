# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "4.2.0"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.0/tish-darwin-arm64"
      sha256 "1bc492bdd51e9b0d609b4505f4b4648174c766a4c164b6cdd250eeb84d9c7f34"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.0/tish-darwin-x64"
      sha256 "dc7f0b1d80fcb559edc43e430a045a0b0132cbff86e5467f49cbde3827dff1a8"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.0/tish-linux-arm64"
      sha256 "1a53f145417ab5cbbe5b60562c43030e708da187e705ab137f4bc999c1370e8c"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.0/tish-linux-x64"
      sha256 "18be72717e67c28f94027190c45dcac8dd3df6baee91b6b5d214a8605ec6e1d9"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
