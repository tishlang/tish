# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "1.7.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.7.0/tish-bindgen-darwin-arm64"
      sha256 "a197e82fdca58bfce8c8dd668b1ceb7e6398113aeaafb42437a1201b2cc13132"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.7.0/tish-bindgen-darwin-x64"
      sha256 "89056544986abd76d537a5a88d129b32ba1e875d737f49a9b4ebbbfd3d903b97"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.7.0/tish-bindgen-linux-arm64"
      sha256 "5224a8d4b610551119c8501aa87941d1caca90f7f015e8a14c82e2307168b62b"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.7.0/tish-bindgen-linux-x64"
      sha256 "639a5b314b14e45264c1fc1bbb1d1eac61babf936bf98917fa978ec011211cbc"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
