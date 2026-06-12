# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.1.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.1.0/tish-bindgen-darwin-arm64"
      sha256 "620cbc0de83c30ce6d2589aaae001e09cc7fccb1b517d308bdc0e4591fa2a9b4"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.1.0/tish-bindgen-darwin-x64"
      sha256 "16cc207a914606761ae119e022fe073d8f5255eacc457c6dfaecae6450984e28"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.1.0/tish-bindgen-linux-arm64"
      sha256 "67ced74fe4e16293dddffae2ae0c88fd7ab8f01dfbab56b3046283b87cbbc6f6"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.1.0/tish-bindgen-linux-x64"
      sha256 "39df1ec0c7f96f3d7edfe0aeca50d40d66e8f9987d743be7a895e4ff94cbca3c"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
