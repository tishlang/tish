# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.36.1"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.36.1/tish-darwin-arm64"
      sha256 "a5a9961c6c7f65bad136f8be0c2a34023229826ea0531b55d4bb6c73f9e3431f"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.36.1/tish-darwin-x64"
      sha256 "470f2355ab9baf3d9a4cb2f0bfc3f38a21e762d13b03ec94b7126d60a25494e7"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.36.1/tish-linux-arm64"
      sha256 "4bfc004c6261ee944fcf8c70381942a3a60465ad2ee286edb851784636a34ce1"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.36.1/tish-linux-x64"
      sha256 "e3fa6b3da9d932e31de57e9e776eec13738e365553ca5138b3dbfcb8ec77c416"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
