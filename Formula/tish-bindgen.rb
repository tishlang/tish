# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.9.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.9.0/tish-bindgen-darwin-arm64"
      sha256 "7d52122659aba5d5d8097906e3f06856f7c620dcd97c9f1e0b030a52ae8930b1"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.9.0/tish-bindgen-darwin-x64"
      sha256 "5ce843e67ddc34cea4538440e50715c7c908c24eb1ab9864d7c77a5c476580de"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.9.0/tish-bindgen-linux-arm64"
      sha256 "96eb7cc79d1bc7eb1f8d3f452a14dec44603a35f08d19e3e3da34cd098ccbc2f"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.9.0/tish-bindgen-linux-x64"
      sha256 "58039864b792cf5d07fca24d3ca489433ec2d38ea6125d13eec3f1e306e273d7"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
