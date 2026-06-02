# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "1.13.1"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.13.1/tish-bindgen-darwin-arm64"
      sha256 "8e32a7a052638ecd36fc118356010dc2a3c9948d36c75e64e4495709a2c50759"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.13.1/tish-bindgen-darwin-x64"
      sha256 "d98a8ec083905944b71c82c5612e672cd619909eceebb0c194ea7b9bc5d74143"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v1.13.1/tish-bindgen-linux-arm64"
      sha256 "3bdf074e591b2717a1317e512bb3cbfdaa24a1071ab79d1bcb1394b758040b82"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v1.13.1/tish-bindgen-linux-x64"
      sha256 "a436b8a7fc1f0212f55ba0552babb8815773772ca4842b09467639a57046fc33"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
