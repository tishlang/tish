# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "4.0.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.0.0/tish-bindgen-darwin-arm64"
      sha256 "9ef021b37a887987761bf2e6eaa8b1be658eb1a06a1dab08723e512382126342"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.0.0/tish-bindgen-darwin-x64"
      sha256 "24f5240bf2fc5cd1d0418d54db48d37badbea90d0c69d89ae7a0e31a49af6eb4"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.0.0/tish-bindgen-linux-arm64"
      sha256 "ed27061902688d3d5587a9c656c9bb0c314703306a6318c9f3fda4b5cfff1b43"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.0.0/tish-bindgen-linux-x64"
      sha256 "a00684e02e5c97634bb1f23c1452feafa8df6ca67d39eaa3b3d59ed8d43e7e9e"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
