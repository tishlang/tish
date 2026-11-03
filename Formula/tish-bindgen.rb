# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "4.2.7"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.7/tish-bindgen-darwin-arm64"
      sha256 "27e2090d12f0c42a2378e6e4c4eca99704052fdcbaabd0f3faf320dfb8efbf57"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.7/tish-bindgen-darwin-x64"
      sha256 "c226db300f0484792a7ebf25db1561154a2f1be14ff421bf98b8c1867d815071"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.7/tish-bindgen-linux-arm64"
      sha256 "790f314ae90ab2bf35c9caefd275790f3b67d86e25ded617afc992136c2c4f48"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.7/tish-bindgen-linux-x64"
      sha256 "6f34ccec596b3f85d2a30b33c7c20e15cc834f7d8a3b10e1a2f2726f7ddf292b"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
