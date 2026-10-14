# frozen_string_literal: true

# Syntax reference: https://docs.brew.sh/Formula-Cookbook
class Cogito < Formula
  desc "Context harness for AI coding agents"
  homepage "https://github.com/Rene-Kuhm/cogito-harness"
  url "https://github.com/Rene-Kuhm/cogito-harness/releases/download/v#{version}/cogito-#{version}-#{os_and_arch}.tar.gz"
  sha256 sha256

  on_macos do
    on_arm do
      resource "tarball" do
        url "https://github.com/Rene-Kuhm/cogito-harness/releases/download/v#{version}/cogito-#{version}-aarch64-apple-darwin.tar.gz"
        sha256 sha256
      end
    end

    on_intel do
      resource "tarball" do
        url "https://github.com/Rene-Kuhm/cogito-harness/releases/download/v#{version}/cogito-#{version}-x86_64-apple-darwin.tar.gz"
        sha256 sha256
      end
    end
  end

  on_linux do
    on_arm do
      resource "tarball" do
        url "https://github.com/Rene-Kuhm/cogito-harness/releases/download/v#{version}/cogito-#{version}-aarch64-unknown-linux-gnu.tar.gz"
        sha256 sha256
      end
    end

    on_x86_64 do
      resource "tarball" do
        url "https://github.com/Rene-Kuhm/cogito-harness/releases/download/v#{version}/cogito-#{version}-x86_64-unknown-linux-gnu.tar.gz"
        sha256 sha256
      end
    end
  end

  def os_and_arch
    if OS.mac?
      Hardware::CPU.arm? ? "aarch64-apple-darwin" : "x86_64-apple-darwin"
    else
      Hardware::CPU.arm? ? "aarch64-unknown-linux-gnu" : "x86_64-unknown-linux-gnu"
    end
  end

  def install
    resource("tarball").stage do
      bin.install "cogito"
      pkgshare.install "README.md", "LICENSE"
    end
  end

  test do
    system "#{bin}/cogito", "--version"
  end
end
