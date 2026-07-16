# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.39.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.39.0/tish-darwin-arm64"
      sha256 "e816be3da58aa093e1937d3e1e38fd67e114b88ac43c71cf3d7853b5ddd90499"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.39.0/tish-darwin-x64"
      sha256 "3fdaf95313da16dfe0f04c6c4cda1c585bed20582e201d3c821a74eeac6cb5ce"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.39.0/tish-linux-arm64"
      sha256 "384f7f5be05f02f4bd5c5d5c2f2ae5479b810b2822f7636daa9e72668e5d8772"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.39.0/tish-linux-x64"
      sha256 "c5e6b7ec112c9ba319701e80426ff025e06ee32115e5ccfef9b9b14b89c89e5d"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
