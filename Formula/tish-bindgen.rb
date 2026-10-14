# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "4.2.3"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.3/tish-bindgen-darwin-arm64"
      sha256 "0dcf3ba62dab64aba4bed37f60cabcff7b9d81fecc10f77e297df0d150dd67d3"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.3/tish-bindgen-darwin-x64"
      sha256 "f8907dd2e4844806917280b31307e21cfa7fff715c8415c2a667f7a92e8a4ef5"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.3/tish-bindgen-linux-arm64"
      sha256 "0680b3bca7b07d222e38167bab4cd944e7a0b8e30289c1fdaf66a3d26860e806"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.3/tish-bindgen-linux-x64"
      sha256 "59f4177d643921aa907d2d50f47ddcfc9cf03420410d6ee6d841864247f46415"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
