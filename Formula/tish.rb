# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.9.1"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.9.1/tish-darwin-arm64"
      sha256 "248137ebe30ea97a429dc448f4bc0899b93ad6a38af362c0254284920c6568d6"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.9.1/tish-darwin-x64"
      sha256 "f880a58a7d97c674be11c15082aaf17bdeed0edf0277e401afe995911a26380a"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.9.1/tish-linux-arm64"
      sha256 "ceb9dc0b7e3adf8eb6ab7bf7f26cabe77b005daee9d3d08c8c36945fba1db741"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.9.1/tish-linux-x64"
      sha256 "59d697e810ee32d8a628e19f04edbf430917f3c1411c5f1da9d3f1ae47b2ac37"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
