# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.10.9"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.9/tish-bindgen-darwin-arm64"
      sha256 "c39c9a71285d94c052d34f05f36a57f1aec69484a674edaee72019e1d45bae35"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.9/tish-bindgen-darwin-x64"
      sha256 "f24e495a955fcb2a6325db8691898ff0370fd96ab7fd1290087dd975c9759577"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.9/tish-bindgen-linux-arm64"
      sha256 "f192869857d095580166a9444f073b168d9da320ac07184dd5a3e43b0505d26f"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.9/tish-bindgen-linux-x64"
      sha256 "82274864ade84f5bd1808a1493e9ad30a33482d95c6cdefbf66ef73ab9b45ad3"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
