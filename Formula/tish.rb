# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.8.6"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.8.6/tish-darwin-arm64"
      sha256 "162ebb1011ce3c8fcec4b68735c3d8c85a6776263afaa3c3fdb54e0768bc5f1e"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.8.6/tish-darwin-x64"
      sha256 "a856d0413a183f995ed38111e0229dccd11b713e7f3fd1629693bded1160bf32"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.8.6/tish-linux-arm64"
      sha256 "aa2ce91bf1f9df48576c91acd274ce56efba3c27cbbfc747f1063fcc8958128b"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.8.6/tish-linux-x64"
      sha256 "d355e31e99557d9bcddff4384f7d89a654c49e7d24193c15115d113527e1d118"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
