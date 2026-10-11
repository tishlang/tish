# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "4.1.4"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.1.4/tish-bindgen-darwin-arm64"
      sha256 "bda0fd15dfe54a6c971d409b2f16b3c4a984d1584387a4087b7154f48ef96194"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.1.4/tish-bindgen-darwin-x64"
      sha256 "7fcf4c20a3ca02ebb44c391bb3334752ff8d6eb629116a9c93c6d862e5477861"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.1.4/tish-bindgen-linux-arm64"
      sha256 "ba3d8ed1e6ec1c2513915ef3cd70b223e97be8f196ca8dc14965e58c707bd5e7"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.1.4/tish-bindgen-linux-x64"
      sha256 "528a9010def7cb8c7acbff903f5ab3869d13a3ea18c6521e0df14eedbea08ad5"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
