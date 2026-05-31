# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.12.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.12.0/tish-darwin-arm64"
      sha256 "516e949952d019070dd9cd6f189723bed5880827c1b9e88a31f1a46896db7680"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.12.0/tish-darwin-x64"
      sha256 "93db713d0c4947da468d83e0aac1ddd76e6ea646f98442d1587b83e246f2c95e"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.12.0/tish-linux-arm64"
      sha256 "47c1089382e6aad1ab262e2ca65aaf0f1f7ea5f3122131060225e6c8073cbffd"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.12.0/tish-linux-x64"
      sha256 "fcef2b0e20c246b801ee1a4e0742e6776f6d8cb833fecf07d15fad101808a19d"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
