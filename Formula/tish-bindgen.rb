# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.15.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.15.0/tish-bindgen-darwin-arm64"
      sha256 "54325eac417adb12fe4b53fe87aa41c3676a5a9630b1eac2e87ddc6ffea87749"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.15.0/tish-bindgen-darwin-x64"
      sha256 "f292b81a2cf4ae81affafddcd07acca9c6c2c2d107c1f70f9d4ce8a23b442820"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.15.0/tish-bindgen-linux-arm64"
      sha256 "98875dba4496385661e7c05a35d57dab835df8969e5e2f60f3c106e283eb1e4a"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.15.0/tish-bindgen-linux-x64"
      sha256 "399f8d9f71c741ca70ab81ab9f21b70f02213a3347338f0a7fe17da23128a301"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
