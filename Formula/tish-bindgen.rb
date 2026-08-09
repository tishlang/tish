# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.3.3"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.3.3/tish-bindgen-darwin-arm64"
      sha256 "48884549025ef3ed0a1115bbd41ae78be3b46da930ba7df3bfde96770a339fbc"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.3.3/tish-bindgen-darwin-x64"
      sha256 "d5446a9bea17d9ca081ebad9e6b4983c1585f98c360e8e665390d27a44ae6077"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.3.3/tish-bindgen-linux-arm64"
      sha256 "017f16d9264bca1f412554b42c2c2d7aaf38640473d2064401fc0521a53cc103"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.3.3/tish-bindgen-linux-x64"
      sha256 "2a1ba77d7d3cbeab1316afd8a9c28bd600d038d469f31c5289f5eed5cfb19ed1"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
