# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "4.2.5"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.5/tish-darwin-arm64"
      sha256 "1bff62cb5ce32df5342e4919c0f8a08fedf8714bb0d8624c553b75384aeebe42"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.5/tish-darwin-x64"
      sha256 "0474bf72723a60003f9014370cb69e82b9f1aceb1ad4fcdb544d04da252bcabd"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.5/tish-linux-arm64"
      sha256 "a20ab5e06bebd4e599164165b8c09e867af87cfb4eaa5c23edc0c9ea0d59b702"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.5/tish-linux-x64"
      sha256 "28517de9e47ff6309c6f1317f5d36f4f5c9df23695c1a565978c63252ac5bc8d"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
