# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.36.2"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.36.2/tish-bindgen-darwin-arm64"
      sha256 "4b4c3ad1cf5f43d75cfdf8552d0670888dafb76b105c85e4ff564dec9e3f5f6c"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.36.2/tish-bindgen-darwin-x64"
      sha256 "f4d3d57dd9a89ec40ab2b68f1afe5f80512cc61cf6f59004fe9693d30a3570c6"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.36.2/tish-bindgen-linux-arm64"
      sha256 "316e97931c40d69e51f1d5ed03ede614ca315d4a80417ec0aeeac51fcf3b9308"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.36.2/tish-bindgen-linux-x64"
      sha256 "b7b1b6092309230540334616651aeab85857bdef012795de0169aba8b2990b12"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
