# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "4.2.1"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.1/tish-darwin-arm64"
      sha256 "aad91e4f2b42288a5f08c1c56b70573212891fff0e7377c3ccd216a528b6462e"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.1/tish-darwin-x64"
      sha256 "5e8d01766a908c4b6eaa7657399dd71dbfeb26b2478e43647df1c95a8eb3cda8"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.1/tish-linux-arm64"
      sha256 "371fd0a63f9d870d5734b511d519d29ae65545a6b00f3c4cf817a4a7184f99fc"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.1/tish-linux-x64"
      sha256 "e8279ec2f177ddec16d4f1b2cb94886d60dd7ce7d39e8ca62d05c353b19008f6"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
