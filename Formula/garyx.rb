class Garyx < Formula
  desc "AI chat gateway — connects Telegram/Feishu/WeChat to Claude/Codex"
  homepage "https://github.com/Pyiner/garyx"
  license "MIT"
  version "0.1.10"

  on_macos do
    on_arm do
      url "https://github.com/Pyiner/garyx/releases/download/v0.1.10/garyx-0.1.10-aarch64-apple-darwin.tar.gz"
      sha256 "295aee872853e2b322d5de127fc035d6439d157ff4741de393c3ddf54c63cfd5"
    end
    on_intel do
      url "https://github.com/Pyiner/garyx/releases/download/v0.1.10/garyx-0.1.10-x86_64-apple-darwin.tar.gz"
      sha256 "693a7da6715e88b8617b50864b07334d141f44e05bf25270d742e53e2575db9f"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/Pyiner/garyx/releases/download/v0.1.10/garyx-0.1.10-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "122f777332a597579555fc12397bcbfc390324a1f1aa4630e4938b23607e6069"
    end
    on_intel do
      url "https://github.com/Pyiner/garyx/releases/download/v0.1.10/garyx-0.1.10-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "ec70fe12da4af370fe54ec831179f52030edbd2a01141fd18592e61717451614"
    end
  end

  def install
    bin.install "garyx"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/garyx --version")
  end
end
