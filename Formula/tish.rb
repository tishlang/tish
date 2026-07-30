# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.0.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.0.0/tish-darwin-arm64"
      sha256 "8e24328a6988629f755011752147ee6929877b13d5cdf81f7abe48fdcc6b0cf5"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.0.0/tish-darwin-x64"
      sha256 "65a56fee75faaa3eb8a451c003d060321b51bd7cdd3f89eb2b144c9d6345a59e"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.0.0/tish-linux-arm64"
      sha256 "b25a71fbe70077363f60066a5cfc06b17f398ddef40357e136c47fd71db6672a"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.0.0/tish-linux-x64"
      sha256 "6258d7ec163ab43afb6e5f0c6c6c45d7b4e6c8c491f0a761b3ecef7d99db158d"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
