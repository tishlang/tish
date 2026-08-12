# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.7.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.7.0/tish-bindgen-darwin-arm64"
      sha256 "59588778ccafc6adfd63005fd07f45e1cc06a7b94336e8ddb341cf41e3b020e9"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.7.0/tish-bindgen-darwin-x64"
      sha256 "287451ec7bc88da76fa4c3b56a4a5562fb8d5b243956cde4ac2cdf563cd2f2a4"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.7.0/tish-bindgen-linux-arm64"
      sha256 "f663f35f35bd2186f34726169c682cafaa44bec6c285f11d38b3c879dddfc407"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.7.0/tish-bindgen-linux-x64"
      sha256 "c9076af7e39f4edad107a51f424497867fa6bb4fb28f87018dfaf9b83d27ed96"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
