# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.16.13"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.16.13/tish-darwin-arm64"
      sha256 "72f490a1d180b90c76b50d20516874b2c9467e6dc8c861bc93b41bbbd87b9af4"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.16.13/tish-darwin-x64"
      sha256 "75720665b0116a86031c318003d6a67678f893ea23d0ac9d91ee6704c8522e4c"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.16.13/tish-linux-arm64"
      sha256 "b79c8bf1de219baff2683c75cc134778adbf4261afaf3243a8d1909219d64036"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.16.13/tish-linux-x64"
      sha256 "43b4c2c06587385d02fcb4ed7fdfe0baeae2c9ee039f0106f1163238c564857b"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
