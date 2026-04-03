class Ccr < Formula
  desc "LLM token optimizer for Claude Code — 60-90% token savings on dev operations"
  homepage "https://github.com/AssafWoo/homebrew-ccr"
  license "MIT"
  version "0.6.4"

  depends_on "jq"

  # Prebuilt binaries — no Rust/LLVM build dependencies, installs in seconds.
  # Each tarball contains the ccr binary + libonnxruntime dylib bundled together.
  on_arm do
    url "https://github.com/AssafWoo/homebrew-ccr/releases/download/v0.6.4/ccr-macos-arm64.tar.gz"
    sha256 "ddfd0be584915dff2c3c7d27ba8b767e34424971488f85782a84e40738d5f60c"
  end

  on_intel do
    url "https://github.com/AssafWoo/homebrew-ccr/releases/download/v0.6.4/ccr-macos-x86_64.tar.gz"
    sha256 "0f473a559facfe6062a35b63ccc9506b9522ba6965c2db5f2b847d0b3bf59a17"
  end

  def install
    bin.install "ccr"
    # Install the bundled ORT dylib and fix rpath so the binary finds it
    dylib = Dir["libonnxruntime*.dylib"].first
    if dylib
      lib.install dylib
      system "install_name_tool", "-add_rpath", lib.to_s, "#{bin}/ccr"
    end
  end

  def post_install
    # Pre-download the BERT model and register hooks automatically.
    # Runs as the installing user so ~/.cache, ~/.claude, and ~/.cursor are correct.
    # quiet_system — don't fail the install if an agent isn't set up yet.
    quiet_system bin/"ccr", "init"
    quiet_system bin/"ccr", "init", "--agent", "cursor"
  end

  def caveats
    <<~EOS
      CCR setup runs automatically during install (hooks + BERT model download).
      If you see hook errors, re-run manually:
        ccr init                      # Claude Code
        ccr init --agent cursor       # Cursor
    EOS
  end

  test do
    assert_match "filter", shell_output("#{bin}/ccr --help")
    assert_match(/\S/, pipe_output("#{bin}/ccr filter", "hello world\n"))
  end
end
