# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "4.2.3"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.3/tish-darwin-arm64"
      sha256 "e7b1cd98433ab78b265ffc1cc8c4553dc9d746d7c146cc0189b688c7da451c1a"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.3/tish-darwin-x64"
      sha256 "7e9189b0dec7be38e82dd31d08a6673090ace69aee949ff4f3b25fe7d1bfeff5"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.3/tish-linux-arm64"
      sha256 "2d830518521c734cdb0fb98c3962ba239ac6291091a8085a58a3fc018726a454"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.3/tish-linux-x64"
      sha256 "5c1606d787aecbb6fb742a969cdcf1eedfde1cbe949cf5a843af800576b0c6f5"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
