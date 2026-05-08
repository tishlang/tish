# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "1.10.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.10.0/tish-bindgen-darwin-arm64"
      sha256 "070516ab27fca9039f0b94de1d9c9ca1939929eb8643b98df0954051c50ffa01"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.10.0/tish-bindgen-darwin-x64"
      sha256 "a96fb0672c1bb368f048ec19041707f95660af998af10b74b9a690e8bc9e388a"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.10.0/tish-bindgen-linux-arm64"
      sha256 "c84bfd65855f2abba782dbea5f110950631e779ff7145c9c4a441ab1b7b6d0b3"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.10.0/tish-bindgen-linux-x64"
      sha256 "e89be633d7ce4db3e8d4d0b51e12eb77beadf02ff49df6ba9c44b1161c144c30"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
