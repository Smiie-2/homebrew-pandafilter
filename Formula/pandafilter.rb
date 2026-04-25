class Pandafilter < Formula
  desc "LLM token optimizer for Claude Code — 60-90% token savings on dev operations"
  homepage "https://github.com/AssafWoo/PandaFilter"
  license "MIT"
  version "1.2.2"

  depends_on "jq"

  # Prebuilt binaries — no Rust/LLVM build dependencies, installs in seconds.
  # Each tarball contains the panda binary + libonnxruntime dylib bundled together.
  on_arm do
    url "https://github.com/AssafWoo/PandaFilter/releases/download/v1.2.2/panda-macos-arm64.tar.gz"
    sha256 "3c1f629536c874b6ea0d36946c6352ecb3ff591137ce73646b52a29ea58fa3f9"
  end

  on_intel do
    url "https://github.com/AssafWoo/PandaFilter/releases/download/v1.2.2/panda-macos-x86_64.tar.gz"
    sha256 "2a84d3fdb080d10bdaf3214ea8989b9bb5d48f82502cff1677b1dfa48ef7d430"
  end

  def install
    bin.install "panda"
    # Install the bundled ORT dylib and fix rpath so the binary finds it
    dylib = Dir["libonnxruntime*.dylib"].first
    if dylib
      lib.install dylib
      system "install_name_tool", "-add_rpath", lib.to_s, "#{bin}/panda"
    end

  end

  def post_install
    # Register hooks for all detected agents (fast — no network, BERT downloads lazily on first use).
    # quiet_system suppresses output and never fails the install regardless of exit code.
    hooks_ok = quiet_system bin/"panda", "init", "--agent", "all", "--skip-model"

    if hooks_ok
      ohai "PandaFilter hooks installed for all detected agents. Run `panda doctor` to verify."
    else
      opoo "Hook setup could not complete automatically."
      puts "  Run manually after install:"
      puts "    panda init --agent all"
      puts "    panda doctor"
    end
  end

  def caveats
    <<~EOS
      Hooks are registered automatically for all detected agents on install.
      Verify your installation:
        panda doctor

      To re-run setup (e.g. after installing a new agent):
        panda init --agent all

      Then restart your coding agent for hooks to take effect.
    EOS
  end

  test do
    assert_match "filter", shell_output("#{bin}/panda --help")
    assert_match(/\S/, pipe_output("#{bin}/panda filter", "hello world\n"))
  end
end
