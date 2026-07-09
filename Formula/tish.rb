# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.36.2"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.36.2/tish-darwin-arm64"
      sha256 "0af8804a49a6d55ed21c7f40dd87e8bdf931f3b2ecf5d64a53c7e90b4fa14734"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.36.2/tish-darwin-x64"
      sha256 "04b686cb0daeb2f6eff5a55608eaee5c8024e026e13e15373108f50bdba49117"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.36.2/tish-linux-arm64"
      sha256 "af498e61ba39136e60b23ccf01138d4ea0a22db8f87b2e8c2fb0658761dd7f3a"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.36.2/tish-linux-x64"
      sha256 "0436dda10bee7802a58797f0e339591d6258a2e5c6e50cd5b55d048613e6d078"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
