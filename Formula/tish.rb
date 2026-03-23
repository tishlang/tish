# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.0.26"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.0.26/tish-darwin-arm64"
      sha256 "6bbb51aa855d7e22e13f797302ed681c60ba383d448ac44b98afd432a7a40fd9"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.0.26/tish-darwin-x64"
      sha256 "d6455ba32314902cf08eb318f8166b744509883cc757ef14a6e71d61179f79de"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.0.26/tish-linux-arm64"
      sha256 "91587dda69a1c124fc0ad7df4c769661fbe61c635edc5246bd016abdb1649f23"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.0.26/tish-linux-x64"
      sha256 "035d2803845ee552db6cc68c8ede7cef7c479ef35161b0e599c3d36dcd3761eb"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
