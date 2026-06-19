# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.12.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.12.0/tish-darwin-arm64"
      sha256 "bf00ca07165a110a86c6606a0416cc2f7be5951eb569ef7c56f5130d0a175588"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.12.0/tish-darwin-x64"
      sha256 "c6f47946126eb03960e631bed95441cc48120b9a0a646018a887ec2d2d412a96"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.12.0/tish-linux-arm64"
      sha256 "9c41bea2ce8ad1d58c1e0ee1382ac677eb3b72a039b5f59ad0e68b7f0186a4e5"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.12.0/tish-linux-x64"
      sha256 "bdf96046b0dde66bb05e5be5777ae9bdc24f7fa44c3f154bde46a855f0de08ea"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
