# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.13.1"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.13.1/tish-darwin-arm64"
      sha256 "da895b26055245064ebcb79e73a2c19252fa5c609d488627bc061867d3c898ea"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.13.1/tish-darwin-x64"
      sha256 "7e43b3fa929210157a397bdd0998abf1dbdecaf7b6b1c8a8bdf84a69724cb255"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.13.1/tish-linux-arm64"
      sha256 "bbb7bb6c046310864aca576c979c63eb26222bce9f56804e9a5cc110a90e6691"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.13.1/tish-linux-x64"
      sha256 "0ec6a23321db515feab803b85d1d567e199bb9a8379b97f3956989ab98793213"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
