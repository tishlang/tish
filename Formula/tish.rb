# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.5.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.5.0/tish-darwin-arm64"
      sha256 "e0a56e6c47eed329e47e795bcfd13fd5a85b1783349cd5a0ba8b97983aaff011"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.5.0/tish-darwin-x64"
      sha256 "b48e9123440421a78a196aac7c070d29e38c7a041f461be5249c4dce08efbd9d"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.5.0/tish-linux-arm64"
      sha256 "c2e4a73a25347f54c16f52c8345f63ff39b809cddb60dea25295156bc6e67dad"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.5.0/tish-linux-x64"
      sha256 "e8d29d030c05064aa4c32180497dd84ca346160c94e26b12d0fd278847f51363"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
