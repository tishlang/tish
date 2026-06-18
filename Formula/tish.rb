# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.9.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.9.0/tish-darwin-arm64"
      sha256 "dd651cd9eafcdec1bcf9a170812ade891ae241661c356d45aca9c4850900b8c2"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.9.0/tish-darwin-x64"
      sha256 "a0d667d42259d80d2847d0546d5c47a7decc1d16f32152f078adf52d8a96223e"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.9.0/tish-linux-arm64"
      sha256 "eab04c81d4457d46a799314a6b02b8b16b6aabc5fa9e5b9e5d9188ffa6bd18e6"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.9.0/tish-linux-x64"
      sha256 "8b12f0978f912d1e0cb498e888e974130bee7e6e9d62027e719388a0411e0bfe"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
