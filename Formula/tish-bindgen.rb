# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.2.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.0/tish-bindgen-darwin-arm64"
      sha256 "1bafede75ee2ead4cd0b09c75ca84692f209d339fceca27429677db9f817b67e"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.0/tish-bindgen-darwin-x64"
      sha256 "4c5806b38692567663642d3c4293a9d383a8a424b0fb6dcb224141688d71a273"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.0/tish-bindgen-linux-arm64"
      sha256 "6975d533ca39a322aa73c0fe20f51c2556bd63ea0994dbd356a32a410b1a6333"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.0/tish-bindgen-linux-x64"
      sha256 "573fabf7a3bcc4c8ecbe2ddc6207c409997d5e3ce857c5609bd261f92da399c0"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
