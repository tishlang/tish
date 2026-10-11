# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "4.1.5"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.1.5/tish-bindgen-darwin-arm64"
      sha256 "b77155611a29eb5c580be3c2ea4b93e5b0dce1f0432ad87861d2e66aeaf72381"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.1.5/tish-bindgen-darwin-x64"
      sha256 "27f55a994742106de5866fd350fbaa4669a2480a279be7761298c8ded4b06e64"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.1.5/tish-bindgen-linux-arm64"
      sha256 "e77764598dffc5b5e89f480f99685aeaf842c137b912c9c548e71f7fab0e0f33"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.1.5/tish-bindgen-linux-x64"
      sha256 "f10fd30203ba1e2e34cf16b193c3d41a19a65a792f222a2de2416f2194086f34"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
