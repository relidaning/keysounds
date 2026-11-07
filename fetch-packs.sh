#!/usr/bin/env bash
# Downloads the sound packs bundled with Mechvibes (MIT,
# github.com/hainguyents13/mechvibes) into ./packs. Safe to re-run.
set -euo pipefail
cd "$(dirname "$0")"
source config/defaults.sh
if python3 scripts/check-packs.py packs; then
    echo 'SKIP sound packs: default pack and audio are present.'
    exit 0
fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
git clone -q --depth 1 --branch "$KEYSOUNDS_PACK_TAG" --filter=blob:none --sparse "$KEYSOUNDS_PACK_SOURCE" "$tmp/mv"
[[ "$(git -C "$tmp/mv" rev-parse HEAD)" == "$KEYSOUNDS_PACK_REV" ]] || { echo 'Sound-pack source revision mismatch.' >&2; exit 1; }
git -C "$tmp/mv" sparse-checkout set "$KEYSOUNDS_PACK_SUBDIR"
python3 scripts/check-packs.py "$tmp/mv/$KEYSOUNDS_PACK_SUBDIR" || { echo 'Downloaded sound packs have an incompatible or incomplete layout.' >&2; exit 1; }
mkdir -p packs
for pack in "$tmp/mv/$KEYSOUNDS_PACK_SUBDIR"/*; do
    [[ -d "$pack" ]] || continue
    mkdir -p "packs/$(basename "$pack")"
    cp -rn "$pack"/. "packs/$(basename "$pack")/"
done
python3 scripts/check-packs.py packs || { echo 'Existing default pack is incomplete; existing files were preserved.' >&2; exit 1; }
echo "installed $(ls packs | wc -l) packs into $(pwd)/packs"
