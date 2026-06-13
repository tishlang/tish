# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.2.4"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.4/tish-bindgen-darwin-arm64"
      sha256 "80fece4a629a982b8b980a0ac4f15d552cf056682be6ccca3e8511f5c3448dee"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.4/tish-bindgen-darwin-x64"
      sha256 "775009f037bcb16e69f90d7250d9f1a39653c0b5c13ab2718934c251ff98aa90"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.4/tish-bindgen-linux-arm64"
      sha256 "b0362d1b424b7fa223e20fbf04fd70e691d1da28d52240c17f7e2846da9a4194"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.4/tish-bindgen-linux-x64"
      sha256 "7a29ce839d0ff424842482aca036dc1d336be3ce963d1db09075c675948795a5"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
