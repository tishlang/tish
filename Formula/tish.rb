# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.0.33"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.0.33/tish-darwin-arm64"
      sha256 "471f2edf48f0a82413fb466619ef84c0d6a4f4070a9dcf8a7d1b7cc360780994"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.0.33/tish-darwin-x64"
      sha256 "9b675433e6c5bc7b9d799d4f5cb156d5550e535a8405bd6560ac0197c660a1eb"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.0.33/tish-linux-arm64"
      sha256 "b7d38a858b16b146da1938e641f28fc9a58df386f68d45a97cad0a75fd460ed5"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.0.33/tish-linux-x64"
      sha256 "b864a7b58e8e0bff9e104ec199660ec6616d14302475e7ddbd346e6498f3ee03"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
