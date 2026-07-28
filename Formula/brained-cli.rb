class BrainedCli < Formula
  desc "The brained-cli application"
  homepage "https://github.com/Nawy/brained.git"
  version "0.1.13"
  if OS.mac?
    if Hardware::CPU.arm?
      url "https://github.com/Nawy/brained/releases/download/v0.1.13/brained-cli-aarch64-apple-darwin.tar.xz"
      sha256 "9cd52931011f6eff570ef98c715089aba02714770aba09384778d16b6c1dddc8"
    end
    if Hardware::CPU.intel?
      url "https://github.com/Nawy/brained/releases/download/v0.1.13/brained-cli-x86_64-apple-darwin.tar.xz"
      sha256 "c8af72551c6320955f3fd8939d7337ce8cc50783475c6f6bdaebeb2a29673a6e"
    end
  end
  if OS.linux?
    if Hardware::CPU.arm?
      url "https://github.com/Nawy/brained/releases/download/v0.1.13/brained-cli-aarch64-unknown-linux-gnu.tar.xz"
      sha256 "1b144217d733637f09dbd931cc53ea3d3a94ea91507f6bab3e91a48ca078dc1e"
    end
    if Hardware::CPU.intel?
      url "https://github.com/Nawy/brained/releases/download/v0.1.13/brained-cli-x86_64-unknown-linux-gnu.tar.xz"
      sha256 "b5e6249035d8889f7dc9ae7381dcec749030205347d5526e559ee939a0205703"
    end
  end

  BINARY_ALIASES = {
    "aarch64-apple-darwin":      {},
    "aarch64-unknown-linux-gnu": {},
    "x86_64-apple-darwin":       {},
    "x86_64-pc-windows-gnu":     {},
    "x86_64-unknown-linux-gnu":  {},
  }.freeze

  def target_triple
    cpu = Hardware::CPU.arm? ? "aarch64" : "x86_64"
    os = OS.mac? ? "apple-darwin" : "unknown-linux-gnu"

    "#{cpu}-#{os}"
  end

  def install_binary_aliases!
    BINARY_ALIASES[target_triple.to_sym].each do |source, dests|
      dests.each do |dest|
        bin.install_symlink bin/source.to_s => dest
      end
    end
  end

  def install
    bin.install "brd" if OS.mac? && Hardware::CPU.arm?
    bin.install "brd" if OS.mac? && Hardware::CPU.intel?
    bin.install "brd" if OS.linux? && Hardware::CPU.arm?
    bin.install "brd" if OS.linux? && Hardware::CPU.intel?

    install_binary_aliases!

    # Homebrew will automatically install these, so we don't need to do that
    doc_files = Dir["README.*", "readme.*", "LICENSE", "LICENSE.*", "CHANGELOG.*"]
    leftover_contents = Dir["*"] - doc_files

    # Install any leftover files in pkgshare; these are probably config or
    # sample files.
    pkgshare.install(*leftover_contents) unless leftover_contents.empty?
  end
end
