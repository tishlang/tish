# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "4.2.4"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.4/tish-darwin-arm64"
      sha256 "0aee0db7a916e60cd5e1d2278f7c3e8d7da434ba92153bd96259c20a5173b8e9"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.4/tish-darwin-x64"
      sha256 "72f65c091dc2250c0a0baf53b0dcb3c0d7a574763462af44cfe60c64a6ed4d7a"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.4/tish-linux-arm64"
      sha256 "bf4e4b0096935c1aa88fca1588077004a0cdf32ca97ff46709d91408d782f41b"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.4/tish-linux-x64"
      sha256 "2243e3ad2d766a96ba76ae902738dc4aea97c4112ee18ce26c32537915910c93"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
