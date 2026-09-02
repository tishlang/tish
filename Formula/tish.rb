# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.10.9"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.9/tish-darwin-arm64"
      sha256 "ce45b16263c80d31aa7f445f07b22ce141747ffcc8a791165c7fd670717d9730"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.9/tish-darwin-x64"
      sha256 "8657f3f242d7e86ec574fbca7c340b5ebc983e166b709e8ec1eb8a46a3300d54"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.9/tish-linux-arm64"
      sha256 "ab8e171b2703226bf8ff078c9418f96d388b0b329dff46876ca3a188b9124087"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.9/tish-linux-x64"
      sha256 "b9340e7e1659c4528c0c2ea5a924e87d1565f332c60a86e848779b473468df51"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
