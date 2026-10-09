# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "4.1.1"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.1.1/tish-bindgen-darwin-arm64"
      sha256 "5296271739b5729debf4536213229add56ded5bc9ac56b3c4ee67ebbe43e5b52"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.1.1/tish-bindgen-darwin-x64"
      sha256 "a4910d9d2658e741def85079a1c66fa79dadb95ca218f120be7a3acd1a9531aa"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.1.1/tish-bindgen-linux-arm64"
      sha256 "90dce959291397fc25db3732f73c0f42683e39efbb411f8385a8943df33f564d"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.1.1/tish-bindgen-linux-x64"
      sha256 "2aecf6f736cc2156bcda4dbd65b42a9ae63bcc49da8d75ce22d88785b59aa0df"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
