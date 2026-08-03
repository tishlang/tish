# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.2.2"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.2.2/tish-bindgen-darwin-arm64"
      sha256 "704fccc0e5dc086d5df548ca6e4968ffa7f0ea2f1c09822df75cf9602dd6e03c"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.2.2/tish-bindgen-darwin-x64"
      sha256 "d2956daf40fe9e01473e86e360b763f4c3bd08f6ffbfa14354c58d7d92df90a7"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.2.2/tish-bindgen-linux-arm64"
      sha256 "bcfe533919dec2ea46f7e558862960ec8ad1eec5c6b291c8223bd9a299d3b053"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.2.2/tish-bindgen-linux-x64"
      sha256 "a678c53315761a7f170e115b16e32dd1f5d44af90731219706b44233d08e5a08"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
