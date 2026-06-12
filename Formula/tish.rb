# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.0.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.0.0/tish-darwin-arm64"
      sha256 "a2dbb169d22d234d63aefd13a0b7584311b1b21457f6c36a22ee8a7e02dde175"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.0.0/tish-darwin-x64"
      sha256 "6cb0ba38f9a52211c2b86f6b0ee2e50fb06036f879f5e79f976592f9a424126c"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.0.0/tish-linux-arm64"
      sha256 "9a733b28107d5a76650903087db8cc40f669e1fcbd4ff54b576ec82daf078f63"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.0.0/tish-linux-x64"
      sha256 "369c232f9d03a5948112c9eeba9261e335d41fc5a888c1f7b4e80fffafa1a81c"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
