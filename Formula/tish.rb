# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "4.1.1"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.1.1/tish-darwin-arm64"
      sha256 "cd708bb3abe444a3e6e8b5f46d8d681ce338498dbe182f3bad07ef1be93d6200"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.1.1/tish-darwin-x64"
      sha256 "7fd436be449608f7d4e3fc6b85def9abba20277659efe56a085e182bfa0c8e3f"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.1.1/tish-linux-arm64"
      sha256 "369d414301c7e41c0a68cd3b915d8996810b3658bd9247f72b100fa71e73541c"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.1.1/tish-linux-x64"
      sha256 "abddfa1f58de8c9cd7368059cdd57d69a7d5adf4d1597b59aa942671c180f188"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
