# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "4.2.1"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.1/tish-bindgen-darwin-arm64"
      sha256 "06a1751448a580e77546d3e0cdf8e3acd57422ca4ef3174bcc0a2c98e398d6d9"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.1/tish-bindgen-darwin-x64"
      sha256 "9400cda0c41fa3ca8e34e79a8f912f8659fb79c2a11555af1f01f07c60bc0da8"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.1/tish-bindgen-linux-arm64"
      sha256 "b5d8df5ed30533c64b832901aaaaaadfd9337dc28db3eaaa85bff66e49170633"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.1/tish-bindgen-linux-x64"
      sha256 "6b2e0228690ad87bd8aae0f11db56a228619996183cf96c97c9756d0504d78b7"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
