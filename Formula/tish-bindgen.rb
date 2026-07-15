# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.37.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.37.0/tish-bindgen-darwin-arm64"
      sha256 "2b561ea523ea534e21fd9ea7a312302e5b5119b46075b8736b29ba25ae3a4d8a"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.37.0/tish-bindgen-darwin-x64"
      sha256 "f29a92e0c1b370d07cee48b053b96869a316fc2f244c936d1842d292f9cb02ab"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.37.0/tish-bindgen-linux-arm64"
      sha256 "3c7741dee9dc8619f3441d89b17b9dca15622e474fd866e27f586463a45da6c6"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.37.0/tish-bindgen-linux-x64"
      sha256 "1b9c830378a05cb0bffb88f8942eba3438c2fe8a7f8f6af20384be241739f314"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
