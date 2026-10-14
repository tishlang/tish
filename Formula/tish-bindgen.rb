# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "4.2.4"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.4/tish-bindgen-darwin-arm64"
      sha256 "00fe6b86c0b8112ac9363c9b75e2976aa1ad57bb7e0a7b322ffb6619d5201fae"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.4/tish-bindgen-darwin-x64"
      sha256 "617dc7f1a4734138f5eb4c05e43aae8643b30078c58176efc25d36124b5b9bb8"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.4/tish-bindgen-linux-arm64"
      sha256 "7900d59797d1548938940cd25c83739dde17a7315b2c00d59885343cd8617a48"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.4/tish-bindgen-linux-x64"
      sha256 "9b7657033bff6cdcf9c588cf7024e68ad9cc670f0f5adac7d220972a5fdeb9b1"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
