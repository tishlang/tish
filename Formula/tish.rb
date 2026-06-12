# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.0.2"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.0.2/tish-darwin-arm64"
      sha256 "65e32225363cdc94cfc460453a2739af3e49b005318abb3e8eeb3957688e1031"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.0.2/tish-darwin-x64"
      sha256 "99f920be1e299f3f3fe7e00c6b6866e714852aee5e9120a6abb13be621244ba7"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.0.2/tish-linux-arm64"
      sha256 "00e9b8c79222f34b01c71cfcda7e7bc27c1aad99974c0fed8340f6f017993ed7"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.0.2/tish-linux-x64"
      sha256 "f1f4c9c2da16778bc0b92115ed0e6cd7be95f4fbde76089cbfdf4b16958f6c76"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
