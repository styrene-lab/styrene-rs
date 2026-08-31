# typed: false
# frozen_string_literal: true

class Styrene < Formula
  desc "Reticulum mesh network daemon and terminal UI"
  homepage "https://github.com/styrene-lab/styrene-rs"
  license "MIT"
  version "0.1.0"

  on_macos do
    on_arm do
      url "https://github.com/styrene-lab/styrene-rs/releases/download/#{version}/styrene-#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER"
    end

    on_intel do
      url "https://github.com/styrene-lab/styrene-rs/releases/download/#{version}/styrene-#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "PLACEHOLDER"
    end
  end

  on_linux do
    on_arm do
      url "https://github.com/styrene-lab/styrene-rs/releases/download/#{version}/styrene-#{version}-aarch64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER"
    end

    on_intel do
      url "https://github.com/styrene-lab/styrene-rs/releases/download/#{version}/styrene-#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "PLACEHOLDER"
    end
  end

  def install
    bin.install "styrened"
    bin.install "styrene-tui"
  end

  def caveats
    <<~EOS
      Two binaries are installed:
        styrened     # mesh daemon (runs in background)
        styrene-tui  # terminal UI

      Quick start:
        styrened                    # start the daemon
        styrene-tui                 # launch the TUI

      For the Styrene desktop app, see:
        https://github.com/styrene-lab/styrene-ui

      To connect to the Styrene Community Hub:
        mkdir -p ~/.config/styrene
        cat > ~/.config/styrene/config.toml << 'CONF'
        [[interfaces]]
        type = "tcp_client"
        enabled = true
        host = "rns.styrene.io"
        port = 4242
        name = "styrene-community-hub"
        CONF

      Documentation: https://github.com/styrene-lab/styrene-rs
    EOS
  end

  test do
    assert_match(/styrene|identity/, shell_output("#{bin}/styrened --help 2>&1", 0).downcase)
  end
end
