# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.11.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.11.0/tish-bindgen-darwin-arm64"
      sha256 "aed523a4a2b9e47807c9eec619acc8a11be4d1c31b9bab0afcd3b9f39bba444e"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.11.0/tish-bindgen-darwin-x64"
      sha256 "50216cbe0d464235d461ebe1d34cbc84c88ebc432a5830371b6cbbbd436802c5"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.11.0/tish-bindgen-linux-arm64"
      sha256 "18bb34a0bd09cf5e228878c183c2161700e22c15a83b8f20d6baaa2c6c76c894"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.11.0/tish-bindgen-linux-x64"
      sha256 "ea5c75649997f0ca1babf21edfb08af1b24049a1450ba3c124d9c0544c55bb9a"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
