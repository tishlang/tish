# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "4.0.1"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.0.1/tish-darwin-arm64"
      sha256 "4bfbe890fa30989407c2350af476aece70b287212d274fc5a4bb486003add77f"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.0.1/tish-darwin-x64"
      sha256 "d0249bca97cb4e5041e56e57486aa4694d1521d432ad7eca38a4d796cb3d432c"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.0.1/tish-linux-arm64"
      sha256 "ab98709c1673ce251e79f511c9f8b0f49c3be04532a42279df43ae68689c4051"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.0.1/tish-linux-x64"
      sha256 "2826a52a2f2269ff8ef4a457a4fbfd713d47bfae036274f01e2c7be78856c016"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
