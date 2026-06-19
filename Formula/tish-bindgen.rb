# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.10.1"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.10.1/tish-bindgen-darwin-arm64"
      sha256 "bd7269d9a493d4316be343df0ac33fd533138d9d74c7c123404b04c65d39ddbe"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.10.1/tish-bindgen-darwin-x64"
      sha256 "ea2d6f1fd28f893ec3f2e3693cd3a602b216794f49aa3d07c7a34f4827e3bcb7"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.10.1/tish-bindgen-linux-arm64"
      sha256 "cb4d5ead1a0d5c4df62deba3bf57d6118f5c8d8ec0154fff246d6b7ba49da290"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.10.1/tish-bindgen-linux-x64"
      sha256 "d7a248958026a7b34d1ae10d9348eef49d6977726805d746aac451231a853f73"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
