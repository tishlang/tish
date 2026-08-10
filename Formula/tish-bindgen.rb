# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.6.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.6.0/tish-bindgen-darwin-arm64"
      sha256 "4ecea2bf7ebcec1a3dd7d315b14d2edafc59ae4d8a3098d6b578e5860a3ce555"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.6.0/tish-bindgen-darwin-x64"
      sha256 "09f47e14c6a56a03250d82f13595f6c60b00e01cd2750967796094bb80427ef0"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.6.0/tish-bindgen-linux-arm64"
      sha256 "d2818af7e0c087597d30d839b7b30d7fa2a532ed0a5e6a9bfaa50191ab8be276"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.6.0/tish-bindgen-linux-x64"
      sha256 "fa43929385b046746d86610e361f9b7ad2b269537319ec1fd1a5c1c0c04b3228"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
