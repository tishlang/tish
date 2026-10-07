# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.15.3"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.15.3/tish-bindgen-darwin-arm64"
      sha256 "9a131bb9a9249b5fdf31431163d98fd92acaea8e89430aba21209f0d78bdf0f4"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.15.3/tish-bindgen-darwin-x64"
      sha256 "bccbf3468d497525238e43dff7287beae73a7e8ad17059a8bf68e4aa70928366"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.15.3/tish-bindgen-linux-arm64"
      sha256 "2fadcea5b22f62e58aa6bcffbd5547e30bb181bb05256a0613da42c84a7fe1bd"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.15.3/tish-bindgen-linux-x64"
      sha256 "2410182c5d2f61480a4d63d490523837f40f0fe32cf9e3eaa327c2b1993d2f70"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
