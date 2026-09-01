# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.10.4"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.4/tish-bindgen-darwin-arm64"
      sha256 "5237db8ec36099d8502c4a8cd69520bde645d2873980c8dfce71c2cbb6bc9691"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.4/tish-bindgen-darwin-x64"
      sha256 "3e40d6edf8d7f67ac8e7b5ccd4dea1aab5a8399f674522adfe2981f8f28a69b5"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.4/tish-bindgen-linux-arm64"
      sha256 "8c55fd1d774fb9b17a87104f6f48c30cdf04fc4c9c5ad4f923bfff869dd81168"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.4/tish-bindgen-linux-x64"
      sha256 "81fa0447e965efc788d56dea8d2424e7185302684144036008b1ab0f6ac2ea7d"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
