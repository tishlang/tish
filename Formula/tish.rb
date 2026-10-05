# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.15.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.15.0/tish-darwin-arm64"
      sha256 "6810335276b2a73d46e327ae3ebe059dbabca0a9440ed049416af60e23c07512"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.15.0/tish-darwin-x64"
      sha256 "6c2977b0fd8da2c50396d7ec09cff154e9ee154e83cf0317cb20f2d2fb0e7f00"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.15.0/tish-linux-arm64"
      sha256 "8ec7ea7dfc801f87c718214051f767c223e2b3c46618c97c6d8e5f0463e2e44f"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.15.0/tish-linux-x64"
      sha256 "99c7fb48c5cc9a26d8cbaa7367c9095578c2a910e201c8f0f5183bbe9a60cda1"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
