# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.0.29"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.0.29/tish-darwin-arm64"
      sha256 "aa2981e605c89800b9e4b4c27e8533f379bb405b2a8f394ccec75ab26e310cde"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.0.29/tish-darwin-x64"
      sha256 "4cdb5a45dd7d31aa8372c7e54a8c23a9a5a3b00c03757e026ccf3bc7b0ea5d40"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.0.29/tish-linux-arm64"
      sha256 "781c5cb54c8b41366f823b6478df2550c163a255c276ba3714984e43b2a3a60d"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.0.29/tish-linux-x64"
      sha256 "b57680b5be3330935bc07f98ffd30260cb6df9cd5e3097a3de8d1800dbf0d488"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
