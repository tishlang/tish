# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "2.37.3"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.37.3/tish-bindgen-darwin-arm64"
      sha256 "8471c24cb6baffa14089d8f9f4c91ff5dcf715ad19df440d5ea834d1ab6b6fb9"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.37.3/tish-bindgen-darwin-x64"
      sha256 "27a0238b011486a98b04dfec98d83ab425838c5c1b4c5e40217166da7c5b5f09"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.37.3/tish-bindgen-linux-arm64"
      sha256 "dd1452fd19d82295c815d74918a4e171d6ef8f0ace9290ce6ec17594d39fa311"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.37.3/tish-bindgen-linux-x64"
      sha256 "a2896ecac05dfdd938109dab7b055fdf790c38ee06fbc688dbfc39da30a31a79"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
