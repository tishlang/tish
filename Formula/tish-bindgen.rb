# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.2.7"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.7/tish-bindgen-darwin-arm64"
      sha256 "a7129f65d8a555ef66fa6ca37b5c005bb07ddea87b1c527cb1b7ceeded5d4542"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.7/tish-bindgen-darwin-x64"
      sha256 "ddd908877894024535392419d3c8167230fb10b1113563fc8689503c49e78a0a"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.2.7/tish-bindgen-linux-arm64"
      sha256 "7b61091dc1ece3dd4d6a9d53ed862f1fd182fad8b63499d57badf25b7c9b6309"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.2.7/tish-bindgen-linux-x64"
      sha256 "033d27fcfb1880ba4d706b1fb98c1db9b39f9b6a7ecaa91793a0d196ddb953de"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
