# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "4.2.7"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.7/tish-darwin-arm64"
      sha256 "80c676d4d754d926eb3a772c2607094740abe84e8bd6156edf0b926c92bc0f72"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.7/tish-darwin-x64"
      sha256 "0033c53293625b42f83714a088a4dbfa302538c07eb65149707a6f2f5cf0ebe5"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.7/tish-linux-arm64"
      sha256 "24e738f2d13dd17deb4f8b52ac91ca412cb549da68070c1c24a1af1580ae7666"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.7/tish-linux-x64"
      sha256 "89915823ea85fe23c849750e133d5caaef52668813f593cab666380afbc07991"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
