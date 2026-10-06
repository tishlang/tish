# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.15.3"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.15.3/tish-darwin-arm64"
      sha256 "61f4cbe3e49f720ba042eb9006d365ba25f1efe8b0d4f92fccad975427a325d4"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.15.3/tish-darwin-x64"
      sha256 "62a18cb4c2de65e1e5df6a2757bd3beb0deb2a4350aab90cf8908a687fb26fff"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.15.3/tish-linux-arm64"
      sha256 "169692b9eb85549ed630911eb9acb1f5e3aef63f2f4781ce87abc1901e92fb14"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.15.3/tish-linux-x64"
      sha256 "8b783ab36a92b2a022cfedf5a7d5b8e4d50b2c1496ab185b5144751d8100b3ce"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
