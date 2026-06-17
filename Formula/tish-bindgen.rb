# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.8.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.8.0/tish-bindgen-darwin-arm64"
      sha256 "a84828fb1bfa3f2349e71b28d8e6c912cb5934262124e0860eabf21912901d6b"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.8.0/tish-bindgen-darwin-x64"
      sha256 "c6d01fc569e916efbedc1f6634ad90c330676db9bfee81638c0cfb1ae530068e"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.8.0/tish-bindgen-linux-arm64"
      sha256 "1525030461d73e877924ddffb4bd282373988d026cd979d2a24bbca386d85dfd"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.8.0/tish-bindgen-linux-x64"
      sha256 "ef6da5b4cb75897120d79353a3915ad0bdc529199713a78bc3f4f034b0f9fb86"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
