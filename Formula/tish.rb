# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.15.2"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.15.2/tish-darwin-arm64"
      sha256 "e6e2f492ab24a54fa866b3711f8b2c600c661afcb017bbec7b12ea7f1daf7ca0"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.15.2/tish-darwin-x64"
      sha256 "a9e1418eaf5d78282b81fa99e6d027829c1db3173567caaf01f62d634c27bc58"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.15.2/tish-linux-arm64"
      sha256 "24939c5f4efa2a17a9da101db5da973ba5083451e7fa876e01a77fa135fd71aa"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.15.2/tish-linux-x64"
      sha256 "fc95c5b36ddd4f2ae878ad8c676b3c4f27034f7fa8f5ef874e8ea78c2623e08d"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
