# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.0.21"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.0.21/tish-darwin-arm64"
      sha256 "8822884e1f897cdafcebd9c7d643f2bd7b07e1952e057fc2bba60685466674c2"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.0.21/tish-darwin-x64"
      sha256 "996c2f3fda83dc23806de5ff172386de8d2f0085823561778ee3817043b4a853"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.0.21/tish-linux-arm64"
      sha256 "b43b3790e521cf566695586413457163e4891d065f58b4b83d37aa4b2978b499"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.0.21/tish-linux-x64"
      sha256 "2f8d870d8f21c002715c3b29dd1620147755fd360354cfd99eaa2b325ccd504e"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
