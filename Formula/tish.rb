# typed: false
# frozen_string_literal: true

class Tish < Formula
  desc "Tish - minimal TS/JS-compatible language. Run, REPL, compile to native."
  homepage "https://github.com/tishlang/tish"
  version "2.43.8"
  license "PIF"

  depends_on "tish-bindgen"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.43.8/tish-darwin-arm64"
      sha256 "3cf9ab1bd1ef1edced22673e0748a13244bea21b2de0d45e11da2709365947e6"

      def install
        bin.install "tish-darwin-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.43.8/tish-darwin-x64"
      sha256 "bc0fed72f3f3ed3e1f71717089a48f34a4c2abc09220ee17e41b757e02442a39"

      def install
        bin.install "tish-darwin-x64" => "tish"
      end
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/tishlang/tish/releases/download/v2.43.8/tish-linux-arm64"
      sha256 "01f1d540afb2dfb7938a020bdb8f24ad37e441b81672ec0cc92a14b9a9e9874d"

      def install
        bin.install "tish-linux-arm64" => "tish"
      end
    end
    if Hardware::CPU.intel?
      url "https://github.com/tishlang/tish/releases/download/v2.43.8/tish-linux-x64"
      sha256 "51e0e4c3d421697ebd4cb1e0f3e3c10f97e1f4f7a81b62a4e520ccf0128f8c94"

      def install
        bin.install "tish-linux-x64" => "tish"
      end
    end
  end

  test do
    assert_match(/^\d+\.\d+\.\d+/, shell_output("#{bin}/tish --version"))
  end
end
