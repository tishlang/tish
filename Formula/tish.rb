# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.37.3"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.37.3/tish-darwin-arm64"
      sha256 "ad506820557af3b198a825e9278bfff9437708df25da6fa7f328572e86de13f8"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.37.3/tish-darwin-x64"
      sha256 "d927a21c52347e197ae5795fdbe247571f86c6fdc76d97be5802aa0f8932d685"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.37.3/tish-linux-arm64"
      sha256 "1d2eda4e74a30625d18f52fea31cc19512f824f8312d9041ba0fc1e761ff14b0"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.37.3/tish-linux-x64"
      sha256 "413391735eff932e899f40b417b1804fe668d2fbae91b57464e99b1bc333cd13"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
