# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.11.0"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.11.0/tish-darwin-arm64"
      sha256 "0ce432d3cbc881e8754a77d32128da50333a120b4e88b639dc042a5a58ffefe8"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.11.0/tish-darwin-x64"
      sha256 "aa104aaefbe0a7fc5f1f1052cca07bb89db9530cea95e685d969a93a8111c3cf"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.11.0/tish-linux-arm64"
      sha256 "03fcffe5beefe21766dbf4cfd477556920d39883fbdb548d96ae2f408b3ce042"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.11.0/tish-linux-x64"
      sha256 "b7f2c7cfef232d3cacc5e4f4d856103d4f07cb20fca0ee26c8be9a1b612be625"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
