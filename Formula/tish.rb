# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.1.2"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.1.2/tish-darwin-arm64"
      sha256 "d724f9a536960c01e68d598f70c65b168df870bbaf968d7e8d30c7bcfc0c3636"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.1.2/tish-darwin-x64"
      sha256 "e770a57357d993b13ba84dc2d9866a8fde17169e8d0df9ab5d55ed4eca5e1253"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.1.2/tish-linux-arm64"
      sha256 "e36e9ecfdd9b1a455f90f84cd1b89645147e9f94fec657e2c98545d0e7bdd804"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.1.2/tish-linux-x64"
      sha256 "a70a6865dcd1802ec61853e2a66863c9660faee3dbab7402c4da7bb8afa7a688"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
