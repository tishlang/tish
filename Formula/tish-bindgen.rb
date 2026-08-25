# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.9.1"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.9.1/tish-bindgen-darwin-arm64"
      sha256 "b73fa4e85d07486c3788e59a2ea0eb7392c1030ddfa2c792d0d056de864f54d1"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.9.1/tish-bindgen-darwin-x64"
      sha256 "5120a2f46962a85e882c573a294f297fb7d80fda025f291fc2539db09841ef01"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.9.1/tish-bindgen-linux-arm64"
      sha256 "81cc01090ac4ba60e8d325a362d23a0165a8197d2451fe2d637e5c6008626d26"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.9.1/tish-bindgen-linux-x64"
      sha256 "0daf1cac5e606856ad461d4c7683a0f7eedf56fea3bc29dd42d6ae91298d662d"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
