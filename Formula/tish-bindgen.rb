# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "4.2.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.0/tish-bindgen-darwin-arm64"
      sha256 "b9f49617c962787a53ba1e3e25eaab7277b9090be696be8996c32c8c6ef86a7d"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.0/tish-bindgen-darwin-x64"
      sha256 "b67f0332afe65e633d068f9d61aff1ff514254ec603b221b7c2b69d7225058cc"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.0/tish-bindgen-linux-arm64"
      sha256 "19f429bacd5615fb6f6a5d7a5da0469fd8e552e504510897e09515195a760a87"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.0/tish-bindgen-linux-x64"
      sha256 "1af032aed7b4d536168b2e2fd840c7d472d7f42dc7a16adc1d13d3271f9b9a66"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
