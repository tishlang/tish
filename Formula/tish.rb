# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.13.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.13.0/tish-darwin-arm64"
      sha256 "cbca5a50c7c666735130ea7e55d878cf3e38188507441cea3f0e187bfd4d8b2d"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.13.0/tish-darwin-x64"
      sha256 "a0f2a3703a0513e4bcd5e2db4d51cf2eef1f1631b03deaec47a5e87c43d7d215"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.13.0/tish-linux-arm64"
      sha256 "5ebfb604d309431d54b39cc33ad35621442290de68308a762a2e76fbc0f3ff89"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.13.0/tish-linux-x64"
      sha256 "bd8cb4e8cc7b3523eb1d55af690701ca48d580d79f123c5d3db5f9277e3a26b0"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
