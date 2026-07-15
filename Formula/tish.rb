# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.37.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.37.0/tish-darwin-arm64"
      sha256 "42953935bfa4d70f9ef1c41962e204524c8dd6fbc3c626a8549664397833ab19"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.37.0/tish-darwin-x64"
      sha256 "2ca08d300fc956b720cebf9d0af645ef096159edeea46a07cbd048bcab85ea06"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.37.0/tish-linux-arm64"
      sha256 "8c99191c7863faae4f8bb4d794970b07fdd6016f48d8767baab0a38661186e28"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.37.0/tish-linux-x64"
      sha256 "63f0e8fe133dbae8042ed05280d521ad177a96cfdf466ed739319249e5e02003"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
