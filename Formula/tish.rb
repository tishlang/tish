# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.3.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.3.0/tish-darwin-arm64"
      sha256 "b9ff55dd773d0c9995b242c8d03cdc48dd66e7995f09dfd8e7e62d2c7f89063c"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.3.0/tish-darwin-x64"
      sha256 "cbf2e80ad844e2857981937eab2f7039ba5ca34c9a697891d6d890fcb460f925"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.3.0/tish-linux-arm64"
      sha256 "e0a9ca4ea4aeec149a7443e8bd87a1ccf36856feadd02d59053dacee9a3130a5"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.3.0/tish-linux-x64"
      sha256 "4c9328e793eb66433c1ea71fb184c5d4dac49ffdcd30c9005c5cddd6d6b55a80"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
