# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.15.1"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.15.1/tish-bindgen-darwin-arm64"
      sha256 "872ff17ca8ab10c774c1cadb1d451ecc843d48bd111f7906aefeb6802b1ac5f5"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.15.1/tish-bindgen-darwin-x64"
      sha256 "356c665bbbeb19650db83858d90d925ef0cee42696b6c32313ca706d6891e465"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.15.1/tish-bindgen-linux-arm64"
      sha256 "1b5f342ad682580384e2714f71115a028669a27d33dc2deb1f64b94f99a04c64"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.15.1/tish-bindgen-linux-x64"
      sha256 "30bd25e61f4f084f32e555b10e912ab7498a8dcf348a367bb0ad99918430396d"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
