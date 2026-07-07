# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.35.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.35.0/tish-bindgen-darwin-arm64"
      sha256 "10da2b38fc6c766c094ba4054a9cd6ab8aec075b245a867f6cdc7b2e38a8cc17"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.35.0/tish-bindgen-darwin-x64"
      sha256 "0b83be9872cc741bd49c006b278d17a2c2ed89752d96484e27f359cab5613476"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.35.0/tish-bindgen-linux-arm64"
      sha256 "85f81313c522cbdc066244638cadc8d3811c4d9ad32f3fdd790ba42de862994d"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.35.0/tish-bindgen-linux-x64"
      sha256 "eaa93ffda804c35ba58e8e3813a3326fca07c06254afbc3f59ff1b24c0618b4e"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
