# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.6.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.6.0/tish-darwin-arm64"
      sha256 "9e1d6b3fe0d0ee14704763a22277d45eb3e29e85851bf7b938215482601c3f75"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.6.0/tish-darwin-x64"
      sha256 "a39ab389b1841d8de35a3f6ec9d7fddf011225be1b4379ede32c90e8933d6b97"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.6.0/tish-linux-arm64"
      sha256 "e72633d33642aa6169af2cb076bfbe460f3923569701ff049f980e4d1a440093"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.6.0/tish-linux-x64"
      sha256 "284f0939a60c620e5d967edfb77a1efca5b60d5ba303c8844fea22ceb0ad511c"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
