# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "1.9.1"
  license "MIT"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.9.1/tish-darwin-arm64"
      sha256 "4668a9c48c91b2ed8cc91f64f8111353a7e1120e794878ea21b2cd6a4e8646cc"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.9.1/tish-darwin-x64"
      sha256 "91cae6bdea87f8ba551a440f5168c07b635fb9fdb02c6b61179901d31419b5da"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.9.1/tish-linux-arm64"
      sha256 "650a4ffdb20610d273990f65165121f2696c6b97f024f9ac2faff250ce53faf9"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.9.1/tish-linux-x64"
      sha256 "e880b1d111ff05dbf6864784d7a9eb86d66fbbe3604a4487b59ec6dff0d28d92"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
