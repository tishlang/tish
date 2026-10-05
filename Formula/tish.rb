# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.15.1"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.15.1/tish-darwin-arm64"
      sha256 "2de140761e0b8cd5156e1dba1f53548c5b8da0b87da320beaaf6e19731efb379"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.15.1/tish-darwin-x64"
      sha256 "1b768ede5bb6d17e625bab68ccf0f4ca241902bd9ba31fb4963014ee4e800998"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.15.1/tish-linux-arm64"
      sha256 "95782145da8c7113ab3f8d0270c8ac585e61d84914c2537c59232cf4798e69ab"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.15.1/tish-linux-x64"
      sha256 "e9c856c977b5ee29ab0dca7ce0d71d42a3d49bc004336ff2b72b7c522b9e7f79"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
