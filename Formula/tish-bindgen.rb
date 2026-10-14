# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "4.2.5"
  license "MIT"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.5/tish-bindgen-darwin-arm64"
      sha256 "834e0ed4caf8be0851ba9eedd20bb51863d4826d847119af955e541a7b5e1f22"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.5/tish-bindgen-darwin-x64"
      sha256 "1dcdfbc23107ff3573868e2da586be2c929eee90bd2d055d4e91bca3ed1b9d10"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v4.2.5/tish-bindgen-linux-arm64"
      sha256 "9b3505a536c7b91f396ed41466c6ff733af03e3ebeccf19bf3da1b71a12168fd"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v4.2.5/tish-bindgen-linux-x64"
      sha256 "66514d5ae623c4d713d380e98041162054d3be78f9ced55cdbb16c6176892318"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
