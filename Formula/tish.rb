# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.10.0"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.10.0/tish-darwin-arm64"
      sha256 "e2ed95475425d6b9fac1379404eee5197b221bc0eaa70f80cef26f64ba39485a"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.10.0/tish-darwin-x64"
      sha256 "5f87d526c834f0dbed59491a86f26302ce2e7fd16fb917a6caec3ef5d2eeb77d"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.10.0/tish-linux-arm64"
      sha256 "e8b7783ac43ac465df33a8cba530553e6d610083bd62074f4c4914ddf0efb7b1"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.10.0/tish-linux-x64"
      sha256 "a923d93b1996fe62d6a2b753623906a8884d5031f505780478b3e1498a99b4d0"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
