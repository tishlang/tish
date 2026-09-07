# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.12.2"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.12.2/tish-bindgen-darwin-arm64"
      sha256 "b510fc0c1d1a97e6238383eac0ff8a9ad85c995fc22d71a474d5c670ad948661"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.12.2/tish-bindgen-darwin-x64"
      sha256 "b054a9f0e33bd4e4307effe21f2180a069e3c704a2d8fa64ffd02dcdb2c02569"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.12.2/tish-bindgen-linux-arm64"
      sha256 "4daad66676259155f262b183845de57d50e8b58764b09c373ea6628b9851aff8"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.12.2/tish-bindgen-linux-x64"
      sha256 "ffc337ec200650bbee0a48abc2cd64b3112f12a2a5bf6f11d7680a03d923e20a"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
