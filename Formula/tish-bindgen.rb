# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.14.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.14.0/tish-bindgen-darwin-arm64"
      sha256 "4be5cfa9d6b5d828f74ae17ea4a134ef3f36303735ddbd95705dc703cd8f26a5"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.14.0/tish-bindgen-darwin-x64"
      sha256 "9f5d03c38f1bbb9495a2853f7bf87c7941945e9f4a19a4a4e3ff2b199aa9323e"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.14.0/tish-bindgen-linux-arm64"
      sha256 "8d3922f40e88614bc25eb07232cb7b7983fe6b2d2deb6d4b57acbcc6df6ecfd1"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.14.0/tish-bindgen-linux-x64"
      sha256 "070261428d7ebd8e5ed3323ecb8d6f1d64fa2d62646c98a82fb2081377270263"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
