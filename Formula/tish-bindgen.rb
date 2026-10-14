# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "4.2.2"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.2/tish-bindgen-darwin-arm64"
      sha256 "66042c32090530c556f5f1476320b9dd0d752a637ef287e683d88b3659363ad2"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.2/tish-bindgen-darwin-x64"
      sha256 "0baa1e1fd34d3d0170f8dc6bda234ca0fa680f548609f93d3453e3dece2ab509"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.2/tish-bindgen-linux-arm64"
      sha256 "651804ed637c5f3a9acf275223ad2d954676b5d78df4e60d377a2173b7a6a9a7"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.2/tish-bindgen-linux-x64"
      sha256 "0e745dc8c74cc60dcdda7f739bed14bacfa17474b9538d9f810fe6fac6a727ad"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
