# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.10.2"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.2/tish-bindgen-darwin-arm64"
      sha256 "db56c3ed3c55e77a7c82ca18e3250538ea4d5867520e6f26def895cfbf763097"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.2/tish-bindgen-darwin-x64"
      sha256 "505392d781acf41187b4527b1c879ebd89ee1a7b783ce62f18473a63bf59c9b2"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.2/tish-bindgen-linux-arm64"
      sha256 "b3f394ac30ca6034f4941a719af8b0bd984ce8e5a71851fe7aca2c0012a24abe"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.2/tish-bindgen-linux-x64"
      sha256 "afa93c0bab3aa0e2f3bf953f6cc01a53a31816ee0a709e63b53ead1f5cae6cef"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
