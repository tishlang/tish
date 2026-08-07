# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.3.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.3.0/tish-darwin-arm64"
      sha256 "d5ae54c29c7013c910df87846bc28c4da1276cf13f8f10e69a25108e277cadab"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.3.0/tish-darwin-x64"
      sha256 "fba6d527f4e5ef5bf425e72ed7e8fbd4ae28500d89459a3485ed25d0efa7bebe"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.3.0/tish-linux-arm64"
      sha256 "6dae1d49d8e589b78ccbd38c0576c4c6ffcb7b663941a4e0b3f848d326e4d945"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.3.0/tish-linux-x64"
      sha256 "aecd784f8bfa1383a14b0745631dcbde69aea3a5b8cbfbf539ae6164108b4b3d"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
