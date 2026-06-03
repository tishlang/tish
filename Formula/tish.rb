# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.13.2"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.13.2/tish-darwin-arm64"
      sha256 "5f48a9f25fbdce3ca2ba0b32be92ac60b54cf94517e6b65d063c93456fab591c"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.13.2/tish-darwin-x64"
      sha256 "7b4faa0dc03c81c5aa28598dba1801c92f5de7bdefbe9a7b4f73914d0857bb7a"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.13.2/tish-linux-arm64"
      sha256 "bc65a0d7065bd8b9fa3d57af8546b4dad6afcea37eedd36761a3689d839abf9f"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.13.2/tish-linux-x64"
      sha256 "16c3e41bf31e4d2a1f7ba132f94228b5d4c2b9ce5b9f4fe97a0e87a30f852ea7"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
