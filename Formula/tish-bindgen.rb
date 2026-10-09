# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "4.1.0"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.1.0/tish-bindgen-darwin-arm64"
      sha256 "8d7744e11bc4c187e8fb62770052151e66e2c176190baa59572ceb01533d781e"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.1.0/tish-bindgen-darwin-x64"
      sha256 "38381918e88827be5b31eb800239c850590ff6e7b0d4a7708af8f1c781ba141f"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.1.0/tish-bindgen-linux-arm64"
      sha256 "41be4be290b0d8ae4e563805ccf2713790add3ad9dd0ca3c1effe6c30aa433c6"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.1.0/tish-bindgen-linux-x64"
      sha256 "f1f5ba459a7164f1c63de3195b103915fb88e2a595b8de504d94ae906b4511fb"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
