# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.38.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.38.0/tish-bindgen-darwin-arm64"
      sha256 "128582e831eed1f77a3bfd41dbd77c05ed71a78e8f21ff62637767b5a2a84b82"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.38.0/tish-bindgen-darwin-x64"
      sha256 "962459ecbe7d028ed33a132445237f720731ecaba0bd62a8c930acc7ca6e92cb"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.38.0/tish-bindgen-linux-arm64"
      sha256 "8245566e5bb709f49dbc85b7c111603bb6ea49d14a8e78ac1d017b764e8c5042"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.38.0/tish-bindgen-linux-x64"
      sha256 "787e1e255f5252338b48f4f147a99e909c7f0df8cd1986c96f380b63d051934d"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
