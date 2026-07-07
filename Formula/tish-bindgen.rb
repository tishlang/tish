# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.36.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.36.0/tish-bindgen-darwin-arm64"
      sha256 "401bb96d93a2c13f82f0983c03b011d24fccc4e2672cacd0d756d9ebee537a9e"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.36.0/tish-bindgen-darwin-x64"
      sha256 "0324bbd251a9bd15c8d31041ff9cf241eca13669da689c2aa9b5b59de45d0d16"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.36.0/tish-bindgen-linux-arm64"
      sha256 "43f5dcde4195049327a365b341c334853e212ee951e3216bce51682dd84dcd32"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.36.0/tish-bindgen-linux-x64"
      sha256 "7052e6d8cdb51519193ef7be35a7aa1f499952e26939121d5cad5ed16cb69acc"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
