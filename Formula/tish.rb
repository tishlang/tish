# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.3.2"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.3.2/tish-darwin-arm64"
      sha256 "988b6785c5b6fc07136dd259e941246039d8ae11ac24e61d6af17aefd32df7fb"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.3.2/tish-darwin-x64"
      sha256 "cb44cbf99373dc6e733505f4c96d9c34d4258d3a8cae82d09123c0e6d3667a7d"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.3.2/tish-linux-arm64"
      sha256 "80cbfd4166bd7c2b8deb162cb5a75f38c8b3d657b9d70a43023154a84f2a52c9"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.3.2/tish-linux-x64"
      sha256 "8d87cd59ff8396ad92818eeff65e890931d63c0944c001025425b42dbc49f339"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
