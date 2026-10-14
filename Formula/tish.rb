# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "4.2.2"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.2/tish-darwin-arm64"
      sha256 "43ac8a52a1383ec413f2ab1663abfeaba5590d6258f4793bd7377f9591c8bc64"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.2/tish-darwin-x64"
      sha256 "56edcb2b36e6780e727558b15dd526d18c1b5446f76d608e52936ee79a783fb7"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.2/tish-linux-arm64"
      sha256 "0a982069f6746b258c603024a07cb186a047f3aa2a35c2c72765719d177223cd"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.2/tish-linux-x64"
      sha256 "5fcd6a8820b950885cc62d3f52d2035e0c536b39b87d6a89cf6171e8ce76f3a9"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
