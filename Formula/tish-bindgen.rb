# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "4.0.1"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.0.1/tish-bindgen-darwin-arm64"
      sha256 "a2b7dcb560935638767321aa71424fc760f6f4b41f259b03a5678840fb7ea278"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.0.1/tish-bindgen-darwin-x64"
      sha256 "4e9cbb054c9f824e3f5a651db8c3f74a5e3eea26ea446f28c605ca794597d85f"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.0.1/tish-bindgen-linux-arm64"
      sha256 "dbb0d43a39058da9fd2ab5fc5c66aaf430310293a57b576ab1dd72f1b07a405a"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.0.1/tish-bindgen-linux-x64"
      sha256 "ed1673e8581f8136eba13136897f432439d2278d25456ff93a4dc6079c8b4f1c"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
