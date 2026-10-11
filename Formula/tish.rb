# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "4.1.5"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.1.5/tish-darwin-arm64"
      sha256 "4ee5dd81d1ab86407e6bbc9442ddc7fc48c028bbb12c3b9e0eaf619266218ca2"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.1.5/tish-darwin-x64"
      sha256 "89f950dad0cbd3ec16d959c2e74b0521deb00faafc6faa0d975727146c8346af"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.1.5/tish-linux-arm64"
      sha256 "0976c168cc6b5216a0122139606b4bb4b43b78b9b0f85d92ffa374f1b0bd4037"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.1.5/tish-linux-x64"
      sha256 "d507af1170e115a9a878defb33c45ee022558e6b0ffc369000a3db288a593e92"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
