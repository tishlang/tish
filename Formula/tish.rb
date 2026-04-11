# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.6.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.6.0/tish-darwin-arm64"
      sha256 "43604069e5acc58bfd9866cc211786d0ba0371c99e5c2dab1dc893418b542e88"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.6.0/tish-darwin-x64"
      sha256 "43af992dd3e8513045874b6ce97210ada805ccdad2f354e342aa8515fd9edc3a"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.6.0/tish-linux-arm64"
      sha256 "4933f4115e3261ea75f8f8243beb8b9f8245716a70dcfb20d7de2a07c8282999"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.6.0/tish-linux-x64"
      sha256 "bda65dc71986dcaebafdc52bbf9520b251428f194da6868768ec50115391bfeb"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
