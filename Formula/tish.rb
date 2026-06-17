# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.8.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.8.0/tish-darwin-arm64"
      sha256 "57993bb1fd8051dbc57e360b1f48a26613b2cb6eeae7e169ae18f78fb1e2443f"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.8.0/tish-darwin-x64"
      sha256 "185a4b0c3a14fcf688c1edae3b85030bda21ede28c691398b8182e3586092f39"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.8.0/tish-linux-arm64"
      sha256 "b48b1664573720bff7e93f5b66cef34db6c7dd35d7212f973ca36c8c506c0d7c"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.8.0/tish-linux-x64"
      sha256 "ec40d5e5ad18a94d14022ee9bc2fe98c94fa3b603c0db78fe95880284b510428"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
