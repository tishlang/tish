# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.10.3"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.3/tish-bindgen-darwin-arm64"
      sha256 "1c1ac5c2d9ce53749642905c0fe9c773c6be149dd3d5df086b9c9303fd19fa33"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.3/tish-bindgen-darwin-x64"
      sha256 "ee6cf2ac99b8ae6c4dfdd50da8157760904b8c1be923b6833803ffe18ae56c50"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.3/tish-bindgen-linux-arm64"
      sha256 "c3a95263976780fedd1d125d81ee0b6f598170c77d0e573db15db185353b5d7c"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.3/tish-bindgen-linux-x64"
      sha256 "2aa3bb9988a856db285cbdac2a409328ea14533ed5f4e7fb4c46ed5492fcccc9"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
