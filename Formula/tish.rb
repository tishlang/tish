# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "4.1.0"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.1.0/tish-darwin-arm64"
      sha256 "801967020f2dfa86bb17efb4fc147aa1d8c3e1d0200b032423aa499807de4eb0"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.1.0/tish-darwin-x64"
      sha256 "de0c93e2bc6d5a5106864812952b5327186222d110b9b3c8f91b4bf9811ebafb"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.1.0/tish-linux-arm64"
      sha256 "5fd70aa754dea388c291a757167be36cd15f745bdc64564bbd244ef5890afcea"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.1.0/tish-linux-x64"
      sha256 "aebfe59a357300da4ef7b39693fa6cc7d535ba6b102c8b76565e8dbc7d079434"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
