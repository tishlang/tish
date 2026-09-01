# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.10.4"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.4/tish-darwin-arm64"
      sha256 "80958f5bd5f19d462144c7470c2370442a2339c42be97191873da39cca1c73b1"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.4/tish-darwin-x64"
      sha256 "cb85ab7e727ee3163ecc41688116a2af0da6d54e2f870efc6f712fdcbbefd9bc"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.4/tish-linux-arm64"
      sha256 "d847336c5ca482c2726f98a9572eb998b65fb04205b8f62e0a3525931c918e77"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.4/tish-linux-x64"
      sha256 "5bab1e575001da109ad97347dca4a07c91dee1f13e5cf059775edc0d14eca227"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
