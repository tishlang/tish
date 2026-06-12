# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.2.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.0/tish-darwin-arm64"
      sha256 "f7f95d986d912adf136f0359eab53904a21baab5c7cfeeaa48b649ec5af55aa5"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.0/tish-darwin-x64"
      sha256 "741e864290b296e915cebf9549e35eb6fbd005db255cc43faddbdc30c6b940ac"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.0/tish-linux-arm64"
      sha256 "4d43cfa9a3e6cc385745ce3113da6d410f8dfe25ca186815814d7b9a680c175e"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.0/tish-linux-x64"
      sha256 "79c093eddb069569cda9b1e70a66e4164c05f51cf85295475fa4b68546dfe1f7"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
