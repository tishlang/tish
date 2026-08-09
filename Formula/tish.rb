# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.3.3"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.3.3/tish-darwin-arm64"
      sha256 "3583ee9fdd7b2d55f2b7d4d473735199a18b11539472e9e91f7db637121b5e30"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.3.3/tish-darwin-x64"
      sha256 "2be95e562d0c64108116b0e5b1af818a625ef707fb99f2b0bcfecb1486a167e5"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.3.3/tish-linux-arm64"
      sha256 "3b01336990d4c7c77a6c17350da498df0674583ef0fc4d4e1235577c213db71b"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.3.3/tish-linux-x64"
      sha256 "fa703360cf242f7431d1b4bb1ba09175227db81c5378fe95145a844c859329d7"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
