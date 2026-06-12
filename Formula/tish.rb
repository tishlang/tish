# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.1.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.1.0/tish-darwin-arm64"
      sha256 "1c4b151cb5463c1cf21fa250319d95b647d90e178c6aa0c0d47cb120150c85a6"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.1.0/tish-darwin-x64"
      sha256 "2f174f280f613260c070daa8a583b0fb3e0bee56405c4d241a02d807692f74f4"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.1.0/tish-linux-arm64"
      sha256 "30d184a55b7e356ed1bcd8b4e6b0462d3f7524ee693c89f32615e5c5d561e7ca"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.1.0/tish-linux-x64"
      sha256 "e96c66b1a1ff83ce474b590798bc515ed40e1ea3afa89cce375f06e3f342f68f"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
