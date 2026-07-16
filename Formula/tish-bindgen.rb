# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.39.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.39.0/tish-bindgen-darwin-arm64"
      sha256 "913817f1849d36bd78748ecf6a18b156e362ceaad9c61afbfaf15a637f75d80a"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.39.0/tish-bindgen-darwin-x64"
      sha256 "4b28f3bc46fd47c62a8264c2ebf05d155e800e71c9da6ebdfe0fb37071232adf"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.39.0/tish-bindgen-linux-arm64"
      sha256 "8687c2cf5241f7ac8fdfdcef170706d46425fded0ddfec9d35b3aaddfe796920"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.39.0/tish-bindgen-linux-x64"
      sha256 "2053f9ad4a2b4a8efa986add75b952428f18a0ce5bf921d49dbc91a472ef47fa"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
