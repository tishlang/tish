# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.2.1"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.2.1/tish-bindgen-darwin-arm64"
      sha256 "7471e64a478ad339a7cc0778f8e439e1a7c2caea81cc323f05a6217bc1c17c2a"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.2.1/tish-bindgen-darwin-x64"
      sha256 "41c8213a4835ed6a2ba80808893cc46b21964840d386bc59719bec794975cb1c"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.2.1/tish-bindgen-linux-arm64"
      sha256 "afba09c707b46dde28133637ae55f444080f74b53fda829ff23ee5590d780baa"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.2.1/tish-bindgen-linux-x64"
      sha256 "37e3db8940281ece067d615043e45f8aa90020808c23ccda8a3d1b4e5791f859"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
