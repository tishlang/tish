# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.10.3"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.3/tish-darwin-arm64"
      sha256 "35cd605e61a6875d0840c8a1b816b577a51bbd02883e78f3a8f0f45fde264d94"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.3/tish-darwin-x64"
      sha256 "75df5caaba6a8eb3d59eddbd43feb1361570906305fa6bc3c1fc899e9faedd1c"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.10.3/tish-linux-arm64"
      sha256 "57f6bded950387951897179655e7c1866281155be05fad0581c76c212412544c"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.10.3/tish-linux-x64"
      sha256 "103c624951d59406b50ab215ebe3db0086b8f5b8fb3cd6f39d051985e646a170"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
