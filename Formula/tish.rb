# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.2.4"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.4/tish-darwin-arm64"
      sha256 "8ffd62bae8d921caa1190557740d2ba6589dd557944a998404249889fa4bee70"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.4/tish-darwin-x64"
      sha256 "3c950dd91b04942692c9ba9b3bbfb772862fa2f8f662487240edaeaf10924334"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.4/tish-linux-arm64"
      sha256 "dc0020af3e153b15fb5af189a30070f0600dffa616538d2fdcc61f7ad1af59f7"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.4/tish-linux-x64"
      sha256 "36099288bf589030f28dc0bed70ce226e4d68908956fe104343a46b4937c8685"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
