#!/usr/bin/env bash
# Idempotent install: fetch the sound packs, build, link into ~/.local/bin.
set -euo pipefail
cd "$(dirname "$(readlink -f "$0")")"
REPO=$(pwd)

[[ -f "$HOME/.cargo/env" ]] && source "$HOME/.cargo/env"
command -v cargo >/dev/null || { echo "keysounds: cargo not found (install rustup first)"; exit 1; }
pkg-config --exists alsa 2>/dev/null || {
    echo "keysounds: ALSA headers missing (apt: libasound2-dev pkg-config, arch: alsa-lib pkgconf)"; exit 1; }

if [[ -z "$(ls -A packs 2>/dev/null)" ]]; then
    ./fetch-packs.sh
fi

cargo build --release -q

mkdir -p "$HOME/.local/bin"
ln -sf "$REPO/target/release/keysounds" "$HOME/.local/bin/keysounds"
ln -sf "$REPO/keysounds-toggle" "$HOME/.local/bin/keysounds-toggle"

if ! id -nG | tr ' ' '\n' | grep -qx input; then
    echo "keysounds: you're not in the input group, so it can't read keys:"
    echo "  sudo usermod -aG input \$USER   (then log out and back in)"
fi
echo "keysounds installed: ~/.local/bin/keysounds, ~/.local/bin/keysounds-toggle"
