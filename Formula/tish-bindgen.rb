# typed: false
# frozen_string_literal: true

class TishBindgen < Formula
  desc "CLI to generate Rust glue for Tish cargo: imports (tishlang-cargo-bindgen)"
  homepage "https://github.com/tishlang/tish"
  version "3.1.0"
  license "PIF"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.1.0/tish-bindgen-darwin-arm64"
      sha256 "6a394a56eca0de2a37f6c0edb3f5261170660dea0a80e0f4020ea500a047a258"

      def install
        bin.install "tish-bindgen-darwin-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.1.0/tish-bindgen-darwin-x64"
      sha256 "ad2954d73a818c14194aa5f0084714c4ea3e1b2f4c99a4a1f64f94c3657dc377"

      def install
        bin.install "tish-bindgen-darwin-x64" => "tish-bindgen"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.1.0/tish-bindgen-linux-arm64"
      sha256 "353b42547780f5a5b697f098eeef6641c4243a272b3cb3f701906c85266252c3"

      def install
        bin.install "tish-bindgen-linux-arm64" => "tish-bindgen"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.1.0/tish-bindgen-linux-x64"
      sha256 "c507e1fa6e6fc4338da3840304a24e052264027fc1c6f83e766dd0226606ac09"

      def install
        bin.install "tish-bindgen-linux-x64" => "tish-bindgen"
      end
    end
  end

  test do
    assert_match(/tishlang-cargo-bindgen/, shell_output("#{bin}/tish-bindgen --help"))
  end
end
