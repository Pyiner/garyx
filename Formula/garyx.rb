class Garyx < Formula
  desc "AI chat gateway — connects Telegram/Feishu/WeChat to Claude/Codex"
  homepage "https://github.com/Pyiner/garyx"
  license "MIT"
  version "0.1.7"

  on_macos do
    on_arm do
      url "https://github.com/Pyiner/garyx/releases/download/v#{version}/garyx-#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "b44111b21b126be687748379c83432b03461e06a80ae8dd22c89d00276749988"
    end
    on_intel do
      url "https://github.com/Pyiner/garyx/releases/download/v#{version}/garyx-#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "13d8c16f479018c6af0fa49dba0d89e5d851d55446fc23ade203653716552ea2"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Pyiner/garyx/releases/download/v#{version}/garyx-#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "2c207df716318c98bcf012ebe424b98c69dc730591fb31d3eeec67c4a5806397"
    end
    on_intel do
      url "https://github.com/Pyiner/garyx/releases/download/v#{version}/garyx-#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "d1d2e4c7fe54cabc8adb9bf8934bdbf4f1cb85a3590fb958f8963d58ed4dae41"
    end
  end

  def install
    bin.install "garyx"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/garyx --version")
  end
end
