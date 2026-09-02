# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.10.11"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.11/tish-bindgen-darwin-arm64"
      sha256 "b3d1f42578321c5966ad9a5448b0d19feacdf972a604e0c7f0245b7e115ce811"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.11/tish-bindgen-darwin-x64"
      sha256 "fa50f9986f5b95d74c8d892095211ebde18beff0dcb16f8b90464b5fd71bcd51"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.11/tish-bindgen-linux-arm64"
      sha256 "936c6b98370e681a222c255342d6106515f41b9f4b0b509b03cf0a8cde8b1248"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.11/tish-bindgen-linux-x64"
      sha256 "0db6422b2d525552c3666e97359bb0ad3bf07813a19988b5b10e703d48c4f097"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
