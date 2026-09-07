# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.12.2"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.12.2/tish-darwin-arm64"
      sha256 "cd8cb898e60054c63c0fd36f6ad4c43b5996c2402d25241f03b098e2efaed0bc"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.12.2/tish-darwin-x64"
      sha256 "7079a3b79bc5eb583f726cb5b946458354ffb18049750ef54b3ff3422e1ffa76"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.12.2/tish-linux-arm64"
      sha256 "a2891c2592bdc9ed5c2dda33e251ae152adea6a4da8238594d8c923ae55418e6"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.12.2/tish-linux-x64"
      sha256 "e509401fc95a6d088ac8e26ca7d5dc6779f1e6b9cedf05402087650532d0bc6e"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
