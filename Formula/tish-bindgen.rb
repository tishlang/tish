# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.16.13"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.16.13/tish-bindgen-darwin-arm64"
      sha256 "f570b9975e4713a79e65a81a8a814347fad8e608c097c4b8a2e8ecb217710229"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.16.13/tish-bindgen-darwin-x64"
      sha256 "54eaf874b4fe00bb3aa4aecf357ef8e8caac10100dfe5dc6d287028dd803fe5b"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.16.13/tish-bindgen-linux-arm64"
      sha256 "08b143835cdceefe07e2421d4ce20fd18dff96719b2f85daf7ffec066f259c26"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.16.13/tish-bindgen-linux-x64"
      sha256 "4208aa68b4a00355c9f8ee8214e6db133bfd60ca7172ec127076977a6d0cbe65"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
