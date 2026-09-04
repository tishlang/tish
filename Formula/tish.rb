# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "3.11.0"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.11.0/tish-darwin-arm64"
      sha256 "eb9055ecc12dfcf9f7cb195c37fbc63ff79e45e2e5cc1c7eb7a6e570e2a595cd"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.11.0/tish-darwin-x64"
      sha256 "5ad9b24af72b54a624fc2a1ab12a942133c5ec9667432456a1cf011515c29235"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v3.11.0/tish-linux-arm64"
      sha256 "613775c6c1ed80830be9e7b76d562f998ac0dc0e7516cbda346d28e22d34d896"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v3.11.0/tish-linux-x64"
      sha256 "1039d3a2917a42c8e24393519030a7c8b135942f3aa913d45f564b77631c5959"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
