# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.14.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.14.0/tish-darwin-arm64"
      sha256 "bf7c0eac8954de83d96beddbce056a36f0e4143cb1285b7c9c51b55c66b98286"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.14.0/tish-darwin-x64"
      sha256 "d2cb40728ca9d3370596fd144be4c08928a12b48ffc236542494e4886ec36d7c"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.14.0/tish-linux-arm64"
      sha256 "acb6848b6d50bb928915e1797ee4c18cc17e39ce0c7d76bf583925f5ce3b94b4"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.14.0/tish-linux-x64"
      sha256 "bc4c4d4374fad625d7cde12d598012ab5499f334786778013e6e2ce02d35d85f"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
