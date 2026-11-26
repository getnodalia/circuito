#!/usr/bin/env bash
# Regenerate the app icon files every installer carries, under assets/app-icon/:
#
#   circuito.png                                         1024 px master
#   hicolor/<N>x<N>/apps/dev.danielmiguelt.Circuito.png  16 to 512 px, for Linux
#   circuito.ico                                         16 to 256 px, for Windows
#   circuito.icns                                        for the macOS bundle
#
# Usage: packaging/icons.sh [master.png]
#
# Without an argument the master is drawn from apps/circuito/src/icon.rs (the placeholder mark,
# through the render_icon example). To swap the icon, pass a square master of at least 1024 px
# (or replace assets/app-icon/circuito.png and pass it): every other file is derived from it, and
# nothing else in packaging/ names the picture. Commit the outputs and update ATTRIBUTION.md.
#
# Needs: cargo (only without an argument) and Python 3 with Pillow 12 or later
# (pip install 'pillow>=12').
set -euo pipefail
# shellcheck source=env.sh
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"
OUT="$ROOT/assets/app-icon"
APP_ID=dev.danielmiguelt.Circuito
MASTER="${1:-}"

mkdir -p "$OUT"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

if [[ -z "$MASTER" ]]; then
  (cd "$ROOT" && cargo run --quiet --release --locked -p circuito-app --example render_icon -- 1024 "$WORK/master.rgba")
  MASTER="$WORK/master.rgba"
fi

python3 - "$MASTER" "$OUT" "$APP_ID" <<'PY'
import os
import sys

from PIL import Image

master, out, app_id = sys.argv[1:4]
if master.endswith(".rgba"):
    with open(master, "rb") as f:
        data = f.read()
    side = int((len(data) // 4) ** 0.5)
    if side * side * 4 != len(data):
        sys.exit(f"error: {master} is not a square RGBA picture")
    im = Image.frombytes("RGBA", (side, side), data)
else:
    im = Image.open(master).convert("RGBA")
if im.width != im.height or im.width < 1024:
    sys.exit(f"error: the master must be square and at least 1024 px, not {im.width}x{im.height}")
im = im.resize((1024, 1024), Image.Resampling.LANCZOS) if im.width != 1024 else im


def scaled(size):
    return im.resize((size, size), Image.Resampling.LANCZOS)


im.save(os.path.join(out, "circuito.png"), optimize=True)
for size in (16, 32, 48, 64, 128, 256, 512):
    folder = os.path.join(out, "hicolor", f"{size}x{size}", "apps")
    os.makedirs(folder, exist_ok=True)
    scaled(size).save(os.path.join(folder, f"{app_id}.png"), optimize=True)
# Pillow takes the sizes to embed from the largest picture it is given.
im.save(
    os.path.join(out, "circuito.ico"),
    sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)],
)
im.save(os.path.join(out, "circuito.icns"), format="ICNS")
PY
find "$OUT" -type f | sort
