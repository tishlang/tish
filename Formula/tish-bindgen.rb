# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.0.3"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.0.3/tish-bindgen-darwin-arm64"
      sha256 "2486b25555376adcae00c90312886ddef73ac51fcb85be416faf95abf2265a21"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.0.3/tish-bindgen-darwin-x64"
      sha256 "0df883ba13d9b6e3ce2e1c417e3f14104a2944ad223225f50321061a7cc82ed4"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.0.3/tish-bindgen-linux-arm64"
      sha256 "cdab89337b0131c4361b4c8f03f950b217d30ea54d4575650f5cc178cb630138"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.0.3/tish-bindgen-linux-x64"
      sha256 "8b26c09b34180f204638c2bace010d28fab2e7aab92ad289521998a427abba18"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
