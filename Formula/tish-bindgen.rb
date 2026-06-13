# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.2.3"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.3/tish-bindgen-darwin-arm64"
      sha256 "936a34904feaff6ee897bdec69091803a76f3be40aabe334afe0b6f8ba81e422"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.3/tish-bindgen-darwin-x64"
      sha256 "9d92afc7d458657404d7dc01c926e11a461af4c98781534808f94a87ac644fe8"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.3/tish-bindgen-linux-arm64"
      sha256 "cf4b3c0910d07aa4a4155ac49f04efa8b600fee7b66a16203dcd0c9f75a0355c"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.3/tish-bindgen-linux-x64"
      sha256 "7feb11ef3b7ea3378efde273c2305cec68145fd2cc23e409dcc168afb9585b98"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
