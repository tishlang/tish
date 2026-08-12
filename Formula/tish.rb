# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.7.1"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.7.1/tish-darwin-arm64"
      sha256 "e5c0c14c82a80204267baa05dda2c7547cc4c918f0a41a16f8addba7e8343964"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.7.1/tish-darwin-x64"
      sha256 "f426c9aec5e82eb9a61fecf08ff74b97158d2fa4f6974ee30fd9a1cd582d7490"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.7.1/tish-linux-arm64"
      sha256 "d702ca83318413ca81ba5d1330d32fd888f48724893ffd6f5ad0779064064500"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.7.1/tish-linux-x64"
      sha256 "a95620eab8026e0bd8ae558efc762166395c414e339fe5bdaf3d6698db8d4632"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
