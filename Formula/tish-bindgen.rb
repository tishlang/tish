# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.3.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.3.0/tish-bindgen-darwin-arm64"
      sha256 "04330d99573c555290e5ed987079a91667f6c2cbc375c5ce5acd0c91f7a2afc0"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.3.0/tish-bindgen-darwin-x64"
      sha256 "d4e5c815078f0185646405c40ad90fad07b84998aa775f9a1b796775a6f094f9"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.3.0/tish-bindgen-linux-arm64"
      sha256 "1317a07b0abd69fb4d4c88e847131f40a8834524932513d40dc4486177b9079f"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.3.0/tish-bindgen-linux-x64"
      sha256 "30b8b2b66cf2e6151646efee077e21476ed0945ad5d10fd5b9b7286bc4941c6a"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
