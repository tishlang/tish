# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.36.1"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.36.1/tish-bindgen-darwin-arm64"
      sha256 "ca1f59d9fdf22a435ff0aa63f7b5212df946dffa5c902da4fbe85de313756872"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.36.1/tish-bindgen-darwin-x64"
      sha256 "d6a016776ce8a86442bd8255d42f1182dcad4beec6183bc10076c74c96b4f898"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.36.1/tish-bindgen-linux-arm64"
      sha256 "0d922dcf72cd66a60bf00e0fbcb7ffa185662df2b6ada90ef11dea75d3c50d45"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.36.1/tish-bindgen-linux-x64"
      sha256 "4f588dd5b7fcbb72c91878add4cd9836adc6d89c251808fce4238802986ec450"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
