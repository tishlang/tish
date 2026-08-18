# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.7.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.7.0/tish-darwin-arm64"
      sha256 "b7c567f84cd0ebd79586129e55e74f70deddb91ba01c4a5b0527ea8d00b9f7e6"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.7.0/tish-darwin-x64"
      sha256 "a957518b014527d13d40d22417f29006650a706172b6670270183ed84ae5e996"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.7.0/tish-linux-arm64"
      sha256 "07c1617cbfc0c63fe88b8a0f9962eb7a9f93e6665c2fbee901c8e8f27075d6e7"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.7.0/tish-linux-x64"
      sha256 "48ab590bab22292c7ea401106256ea7417f5734c8527abdb8f7d78eb4f4903fc"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
