# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.0.1"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.0.1/tish-darwin-arm64"
      sha256 "bbe6bb05a809614bc5b6d7e96607a408bd0f9d853a5d57567f8124e454850ea7"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.0.1/tish-darwin-x64"
      sha256 "2586a0ec01beee9ff086f99e5d7842530067931128c51f6c44327b6af2f39d65"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.0.1/tish-linux-arm64"
      sha256 "5df666dd230d18714e05ae57d055c8641fe7192636ac5bc4a35ba171a2935d75"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.0.1/tish-linux-x64"
      sha256 "423c039b23e314ea4a1961e92caaed9be8471f32901db68e6f3d04645fe93950"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
