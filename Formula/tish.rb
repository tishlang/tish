# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.4.2"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.4.2/tish-darwin-arm64"
      sha256 "963719d8ff3b46397970fc7b953178b91e121e79f66850d3d13ff7a893f0b2b7"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.4.2/tish-darwin-x64"
      sha256 "1be6fd4a23ac30d011829631bf380e7032c39e9b6940e4be9a36dc98ab2c83c4"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.4.2/tish-linux-arm64"
      sha256 "2f07a04bd263a7ac4c0dcef3bdffab32adb6668dd6ed948b12234970d2ae0359"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.4.2/tish-linux-x64"
      sha256 "7f22e85293805ad3cfbe5d80cd458d6ff541517b39514c201b78bd08e5ff4f57"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
