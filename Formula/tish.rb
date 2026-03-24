# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.0.28"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.0.28/tish-darwin-arm64"
      sha256 "eee4cf3d8c2731fce1c916af708dd88e71c7fc913315d7041f753bcc7a53d246"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.0.28/tish-darwin-x64"
      sha256 "c54d8af30caa0949ea27913ca0edb6c2e0e2470d87caee2b9cc1d01101193c1b"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.0.28/tish-linux-arm64"
      sha256 "f7858f7d9e2fa9236c14accf396e5e2be303ddf032471cea0c30fdb6b86eb573"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.0.28/tish-linux-x64"
      sha256 "3ab90026b89e9081de6ad7415c377c02abd807bf1ccf1f60c9d408550da47acb"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
