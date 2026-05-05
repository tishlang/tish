# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.9.2"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.9.2/tish-darwin-arm64"
      sha256 "a7d449af68fa30190d81fdc97035f35a0defff8be58eb8c9343d62ddbcfa473e"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.9.2/tish-darwin-x64"
      sha256 "a11fc9b6d987d638decbe697092936f80712156e35386f74d5567dd3de0cda0c"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.9.2/tish-linux-arm64"
      sha256 "3250d0de8b60b2ecba541cd2408319864ec8c0393f42ed084e88be7a8db73398"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.9.2/tish-linux-x64"
      sha256 "e6ae4b64a608b096af6c2689a4bbafc90051a6ed1e9d651bf34725f115d4da63"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
