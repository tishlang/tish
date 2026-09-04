# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.12.1"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.12.1/tish-bindgen-darwin-arm64"
      sha256 "4b4412cbfe5774129e26ac82af8568cad8857d83d26b839ed173b5f0ea69c887"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.12.1/tish-bindgen-darwin-x64"
      sha256 "07cb9ee35a59dac21910ba10f3af338f954702877840e6fbf5825b49f80fe05e"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.12.1/tish-bindgen-linux-arm64"
      sha256 "40c3552494e161a01b2c7206b7fc2274ec2187f668dd1b6fc32c257b4d5bf4d9"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.12.1/tish-bindgen-linux-x64"
      sha256 "04c88d5a2368527e95f438a8043711bf828b784e55ad07384f446a8bf5494c1e"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
