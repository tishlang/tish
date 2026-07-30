# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.0.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.0.0/tish-bindgen-darwin-arm64"
      sha256 "8364692b119ccc83d41934aab2890b4533498675a6666951da9659f8f5ff3cba"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.0.0/tish-bindgen-darwin-x64"
      sha256 "d175c9fb7779ef53f4392cbbbffd5580fd3361f3ae1f283b2a0f7b735ab89121"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.0.0/tish-bindgen-linux-arm64"
      sha256 "4494cd941581e4a2d3b1b2928c32232d360feb905c85fbe53d1f5abb688d4558"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.0.0/tish-bindgen-linux-x64"
      sha256 "5e7dcb955a979c91bb717dc8542d1e933d6e6a71226a00d02a1adc40366676a3"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
