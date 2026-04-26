#!/usr/bin/env bash
# PandaFilter installer
# On macOS: installs via Homebrew (prebuilt binary, takes seconds).
# On Linux / no-brew: builds from source via cargo (~1 min on first run).
set -e

REPO_URL="https://github.com/AssafWoo/PandaFilter.git"
CARGO_BIN="${CARGO_HOME:-$HOME/.cargo}/bin"

# ── macOS: prefer Homebrew ────────────────────────────────────────────────────

if [[ "$(uname)" == "Darwin" ]] && command -v brew &>/dev/null; then
  brew tap assafwoo/pandafilter 2>/dev/null || true

  # Detect the "64" bad-keg: older installs stored the keg as version "64"
  # (inferred from "arm64" in the asset URL). brew upgrade skips it because
  # 64 > 0.5.x. Force a reinstall to fix the keg name once and for all.
  CELLAR="$(brew --cellar assafwoo/pandafilter/pandafilter 2>/dev/null || true)"
  if [[ -n "$CELLAR" && -d "$CELLAR/64" ]]; then
    echo "Detected stale keg (version \"64\") — reinstalling to fix..."
    brew reinstall assafwoo/pandafilter/pandafilter
  elif brew list assafwoo/pandafilter/pandafilter &>/dev/null 2>&1; then
    brew upgrade assafwoo/pandafilter/pandafilter || true
  else
    brew install assafwoo/pandafilter/pandafilter
  fi

  echo ""
  echo "PandaFilter installed. You're all set — hooks are registered automatically."
  exit 0
fi

# ── Linux / no-brew: build from source ───────────────────────────────────────

if ! command -v cargo &>/dev/null; then
  echo "Rust not found — installing rustup..."
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
  # shellcheck source=/dev/null
  source "$HOME/.cargo/env"
fi

echo "Building PandaFilter from source (this takes ~1 min on first run)..."
cargo install --git "$REPO_URL" --bin panda --locked 2>&1

# ── Fetch ONNX Runtime shared library (CPU baseline) ─────────────────────────
# PandaFilter uses fastembed → ort with `load-dynamic`, so a libonnxruntime.so
# must be discoverable at runtime. We drop a CPU build into ~/.local/share/ccr/
# onnxruntime/. Users wanting NPU acceleration can replace this file with an
# OpenVINO-EP-enabled build, or set ORT_DYLIB_PATH.
ORT_DIR="$HOME/.local/share/ccr/onnxruntime"
ORT_LIB="$ORT_DIR/libonnxruntime.so"
ORT_VER="1.20.1"
if [ ! -f "$ORT_LIB" ]; then
  ARCH="$(uname -m)"
  case "$ARCH" in
    x86_64) ORT_TARBALL="onnxruntime-linux-x64-${ORT_VER}.tgz" ;;
    aarch64) ORT_TARBALL="onnxruntime-linux-aarch64-${ORT_VER}.tgz" ;;
    *) ORT_TARBALL="" ;;
  esac
  if [ -n "$ORT_TARBALL" ]; then
    echo "Downloading ONNX Runtime ${ORT_VER} (CPU)..."
    mkdir -p "$ORT_DIR"
    TMPDIR_ORT="$(mktemp -d)"
    if curl -fsSL -o "$TMPDIR_ORT/$ORT_TARBALL" \
        "https://github.com/microsoft/onnxruntime/releases/download/v${ORT_VER}/${ORT_TARBALL}"; then
      tar -xzf "$TMPDIR_ORT/$ORT_TARBALL" -C "$TMPDIR_ORT"
      ORT_SUBDIR="$(dirname "$ORT_TARBALL" .tgz)"
      ORT_EXTRACTED="$TMPDIR_ORT/${ORT_TARBALL%.tgz}"
      cp "$ORT_EXTRACTED/lib/libonnxruntime.so."* "$ORT_DIR/" 2>/dev/null || true
      ln -sf "$(basename "$(ls -1 "$ORT_DIR"/libonnxruntime.so.* | head -1)")" "$ORT_LIB"
      echo "  → installed $ORT_LIB"
    else
      echo "  → could not download ONNX Runtime (offline?). Set ORT_DYLIB_PATH manually."
    fi
    rm -rf "$TMPDIR_ORT"
  fi
fi

# Hint about NPU on Intel Meteor Lake / Core Ultra hardware
if [ -e /dev/accel/accel0 ] && grep -qi "Core(TM) Ultra" /proc/cpuinfo 2>/dev/null; then
  echo ""
  echo "Intel NPU detected (/dev/accel/accel0). For NPU acceleration:"
  echo "  1. Install OpenVINO runtime + an OpenVINO-EP-enabled libonnxruntime.so."
  echo "  2. Replace $ORT_LIB with that build, or export ORT_DYLIB_PATH=/path/to/libonnxruntime.so."
  echo "  3. Set execution_provider = \"npu\" in panda.toml (or env PANDA_NPU=npu)."
fi

# ── Ensure ~/.cargo/bin is on PATH ────────────────────────────────────────────

add_to_path() {
  local rc="$1"
  local line='export PATH="$HOME/.cargo/bin:$PATH"'
  if [ -f "$rc" ] && ! grep -qF '.cargo/bin' "$rc"; then
    echo "" >> "$rc"
    echo "# Added by PandaFilter installer" >> "$rc"
    echo "$line" >> "$rc"
    echo "  → Added cargo/bin to $rc"
  fi
}

if ! echo "$PATH" | grep -q '.cargo/bin'; then
  echo ""
  echo "Adding ~/.cargo/bin to PATH in your shell config..."
  add_to_path "$HOME/.bashrc"
  add_to_path "$HOME/.zshrc"
  add_to_path "$HOME/.profile"
  export PATH="$CARGO_BIN:$PATH"
  echo "  (effective now in this session)"
fi

# ── Register hooks for all detected agents ────────────────────────────────────

echo ""
PANDA_BIN=""
if command -v panda &>/dev/null; then
  PANDA_BIN="panda"
elif [ -x "$CARGO_BIN/panda" ]; then
  PANDA_BIN="$CARGO_BIN/panda"
fi

if [ -n "$PANDA_BIN" ]; then
  if "$PANDA_BIN" init --agent all --skip-model; then
    echo "Hooks registered for all detected agents."
  else
    echo "Hook setup encountered an issue — run 'panda init --agent all' to retry."
  fi
else
  echo "Run 'panda init --agent all' to register hooks once panda is on your PATH."
fi

echo ""
echo "PandaFilter installed. Open a new terminal (or run: source ~/.cargo/env) and you're set."
