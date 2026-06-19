# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.12.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.12.0/tish-bindgen-darwin-arm64"
      sha256 "6efd00a36b4e089acbf32a92c526c7ff65d5284c670408fef136c975ccd84af0"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.12.0/tish-bindgen-darwin-x64"
      sha256 "4cd0474fa9c079b391e955bec557e02e80b374f9c4333826a13d8189c0f49058"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.12.0/tish-bindgen-linux-arm64"
      sha256 "b9ffa8f833dff202a74aeba152cb1e9ab5a36923caf553b6f3109a667cc9e826"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.12.0/tish-bindgen-linux-x64"
      sha256 "c49139b600ddf464e61d408331cabdeae2739e47b8448e866ec40fca09848d6a"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
