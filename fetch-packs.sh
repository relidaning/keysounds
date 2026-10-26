#!/usr/bin/env bash
# Downloads the sound packs bundled with Mechvibes (MIT,
# github.com/hainguyents13/mechvibes) into ./packs. Safe to re-run.
set -euo pipefail
cd "$(dirname "$0")"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
git clone -q --depth 1 --filter=blob:none --sparse https://github.com/hainguyents13/mechvibes "$tmp/mv"
git -C "$tmp/mv" sparse-checkout set src/audio
mkdir -p packs
cp -r "$tmp"/mv/src/audio/* packs/
echo "installed $(ls packs | wc -l) packs into $(pwd)/packs"
