# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.8.0"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.8.0/tish-darwin-arm64"
      sha256 "606f4301f0e70155c0c78391c89ca866548186f513f912f1f2147d4a171b91f8"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.8.0/tish-darwin-x64"
      sha256 "f8efbc118db468a1c4bf66d7c20d58b7b2e5898540db30ced960e8215d86c647"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.8.0/tish-linux-arm64"
      sha256 "4fb53a1a02ce291f871226b24c048e5be7a160410f86e26fff20b289cdedffe4"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.8.0/tish-linux-x64"
      sha256 "3c70130d84ed697cd456256a98a6299854fb2fc947c1d8dcee104c3bdedace5a"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
