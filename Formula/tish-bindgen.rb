# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.43.8"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.43.8/tish-bindgen-darwin-arm64"
      sha256 "3ef5d4dbc4e41ca329854438c72ebddb8a5afe3450f356b8ad22e16922eb5e0e"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.43.8/tish-bindgen-darwin-x64"
      sha256 "32b3c92b7bb65b6ecf5c8821e66d5b2f0f37269ddf00dbbfce8015d2c658e6e3"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.43.8/tish-bindgen-linux-arm64"
      sha256 "59e29c6b2ab930f37469a2324f89f68c660e5dc9e7a66186339b4df9f067533d"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.43.8/tish-bindgen-linux-x64"
      sha256 "54430289d514d1004e1c79c3125ef504b8dcf1d9b2da5f4a37dbd1a03b17c9e2"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
