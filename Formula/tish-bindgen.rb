# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.9.2"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.9.2/tish-bindgen-darwin-arm64"
      sha256 "4e0c2b9b7df8e59857fefdb889c18ca9db9bea5b4b22f4e6afcce8eb686dc29d"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.9.2/tish-bindgen-darwin-x64"
      sha256 "bbc96545245270e04464cf44e81662f73f942fbfc989ee33628ac89c2f207de2"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.9.2/tish-bindgen-linux-arm64"
      sha256 "cacd369a026f09be5466cdfefac26c9aeaffa49e71f22ab699121ab1e98457cc"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.9.2/tish-bindgen-linux-x64"
      sha256 "ea704b5bbe30ce76c64d3be85a92ffd8467f215116e09861ebefb5ce7fdfd0eb"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
