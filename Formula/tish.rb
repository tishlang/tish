# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.38.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.38.0/tish-darwin-arm64"
      sha256 "30f7b132e2ef88c17a72b2dd8f904b48f1032ef4914b2e44bba3becba95ea78a"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.38.0/tish-darwin-x64"
      sha256 "275396616539b2a76b501e9b65c16a1f94ed869fc35374c21732231d7658dedb"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.38.0/tish-linux-arm64"
      sha256 "620cb21560445efdeed165d1c36bd9443ded4d8ac704c85cdd276b5cb4c4b0c6"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.38.0/tish-linux-x64"
      sha256 "60e892bc98b2f2e7306c57bd324dcea9e0c876c463ae1280fae65ab3d7eae046"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
