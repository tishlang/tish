# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.0.27"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.0.27/tish-darwin-arm64"
      sha256 "bcc3c52fa7f3df5c4081ffd0adcb536b9a45c33851ba5f9c30202c9688dd49e1"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.0.27/tish-darwin-x64"
      sha256 "e4ce02013b42f582de052ba9dd0005e7be47e968b367cbb6697e2e1492ca3cfa"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.0.27/tish-linux-arm64"
      sha256 "a800d4acd197b344ea7ef3d97c9d6fd1e993f978d85cd3cd1b5d152f44068cf5"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.0.27/tish-linux-x64"
      sha256 "5a5fbfde03470dca86afc2602238632d9974fe594152bd721b373284c657cc68"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
