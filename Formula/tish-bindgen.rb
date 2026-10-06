# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.15.2"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.15.2/tish-bindgen-darwin-arm64"
      sha256 "64c4012e199b5100679f87dba25abf2cef2d61df185d328d0fc13e94bff0874e"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.15.2/tish-bindgen-darwin-x64"
      sha256 "c2c0a7c6e453a771ec28816a6009a02d462da482a691b6fac4506bd7b5205d74"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.15.2/tish-bindgen-linux-arm64"
      sha256 "5004f25e444632addf6212ebfd6ce8cb99216c8573495bac117b0d2e4c4520a3"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.15.2/tish-bindgen-linux-x64"
      sha256 "707f7ddffb62e4f83bf6594c47113496f09476e2dfe6e5072786c2b546f28b6c"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
