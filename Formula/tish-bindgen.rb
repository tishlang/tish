# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.2.5"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.5/tish-bindgen-darwin-arm64"
      sha256 "b8b1bb8f204c80a83ec6b834a46cc9622bbb10863258cc23eb2207f6679b0a23"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.5/tish-bindgen-darwin-x64"
      sha256 "f774a509b5d98ee06489d8ce08be6e3b5f92b27f4370d79f8b3cba95b20039a4"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.5/tish-bindgen-linux-arm64"
      sha256 "7ba388ee7af219d1d2e71c0b9fc87252df6b22cbfba1efcb7dfe3975c21a98ec"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.5/tish-bindgen-linux-x64"
      sha256 "c5da3f33c89d5fd895d4a64f3f4bdee2688930868f76d2756e5632c4145fc175"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
