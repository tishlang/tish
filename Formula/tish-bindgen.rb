# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.7.1"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.7.1/tish-bindgen-darwin-arm64"
      sha256 "a688a95286400114acdf5e8a50a2b76ef23d9deb1f757efe7c1c2d59d216bba3"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.7.1/tish-bindgen-darwin-x64"
      sha256 "a796374d172ada9ca255f410f071bc2a569a343933537caaa3bf6ad00c2c63b5"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.7.1/tish-bindgen-linux-arm64"
      sha256 "4c565d3652884c8ff261dcf25f7e7a4874f30eb881569ac743ea4de30bd0290a"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.7.1/tish-bindgen-linux-x64"
      sha256 "8f53a639e4595a25620893e7d7791a21ec0863e0be2331424703d75b90179535"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
