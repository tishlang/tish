# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "1.8.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.8.0/tish-bindgen-darwin-arm64"
      sha256 "f8ef92aec9ef6cbba513d706dfa4ced38e52a437e6c2fa3443454d6f64077fed"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.8.0/tish-bindgen-darwin-x64"
      sha256 "f8f1bf08994cd6628e784274d8b123d47fadbd1136ff028042d7acef66614896"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.8.0/tish-bindgen-linux-arm64"
      sha256 "df412f94479899833f45e3396554826f0d26be4c241fd5cc9c5703aa7167c6d0"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.8.0/tish-bindgen-linux-x64"
      sha256 "ba8334d273395549feb85543be597eca100a452b9cf9d4b1bdf17ab753106b0b"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
