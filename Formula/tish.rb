# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.12.1"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.12.1/tish-darwin-arm64"
      sha256 "943c776be5813be3e2147c228d2ce8d69077565b60dbcfc1272c0674a5aa62e5"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.12.1/tish-darwin-x64"
      sha256 "f26b5c03c9f6582bae88df37fb9c49066964c2d42486b19d1d471bb06bbf409d"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.12.1/tish-linux-arm64"
      sha256 "f1798fb89d492e323837e7c650f38f5b10bd9da0063a99fd9d933cdce71d6b14"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.12.1/tish-linux-x64"
      sha256 "580b77c629d3eed95a1356023ff79099ab6905f89891f1da12caad96e8494aa9"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
