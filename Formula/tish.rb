# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.36.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.36.0/tish-darwin-arm64"
      sha256 "6cc0ffeb87c8eeec492051cd945f6295fda4c86efed85d6d12943338cb499574"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.36.0/tish-darwin-x64"
      sha256 "760801f5870b721d26299096e3a2e52505e01fcf5d88fd0938795cc1cfa28349"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.36.0/tish-linux-arm64"
      sha256 "c6b219ffd4de095f7a85bb77cda8653ef217eb841b5de92890e7274e810e2527"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.36.0/tish-linux-x64"
      sha256 "a7ca605ae295420d74ae598ada490d9ae0904e7c788607a1eb6f94e0a72da89e"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
