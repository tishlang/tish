# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.7.0"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.7.0/tish-darwin-arm64"
      sha256 "78b26e97c47b33d55da5213721ffedd5698683119011ad091a5ca63c9fbabf82"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.7.0/tish-darwin-x64"
      sha256 "f33a2cad5df397fc3ee1e72543082fcf4f6b31bdb93dee48564f049a0a69b925"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.7.0/tish-linux-arm64"
      sha256 "40aedf50f0f73227c2924aa8563f00f3767b4c345bc5d99ac6237aa49f888369"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.7.0/tish-linux-x64"
      sha256 "307e75f4ded3520d02148488c530ab68c5e86c9a44ccf111537daf0e3ac3e9ca"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
