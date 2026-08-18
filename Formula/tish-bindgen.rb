# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.8.6"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.8.6/tish-bindgen-darwin-arm64"
      sha256 "e033dd06e1c4ff63a861b6b6036dcd88ecc028005f239f91c3468e56d3a5031f"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.8.6/tish-bindgen-darwin-x64"
      sha256 "f9c2527db3b0408c9791ce7255147223cae01fadd0ace28420de8f91d9eae532"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.8.6/tish-bindgen-linux-arm64"
      sha256 "018c8d51ad0b895857611d80230c70617b3abe58947c10ab4728adc6add8bfa4"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.8.6/tish-bindgen-linux-x64"
      sha256 "7b388121ac96686fb3b52783cfeaf47e746ee18ec8cb11df6e2161c282871bf5"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
