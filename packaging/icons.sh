#!/usr/bin/env bash
# Regenerate the two icon files only the Windows and macOS installers need, under assets/app-icon/:
#
#   circuito.ico    16 to 256 px, for Windows
#   circuito.icns   for the macOS bundle
#
# Usage: packaging/icons.sh [master.png]
#
# The app icon itself is the portrait artwork in assets/icons/ (the 1024 px master
# circuito-1024.png and the Linux sizes under hicolor/, made with nodalia-design's
# tools/app-portrait). Without an argument the master is assets/icons/circuito-1024.png. To swap
# the icon, replace the artwork in assets/icons/ first, then run this. Commit the outputs and
# update ATTRIBUTION.md.
#
# Needs: Python 3 with Pillow 12 or later (pip install 'pillow>=12').
set -euo pipefail
# shellcheck source=env.sh
. "$(dirname "${BASH_SOURCE[0]}")/env.sh"
OUT="$ROOT/assets/app-icon"
MASTER="${1:-$ROOT/assets/icons/circuito-1024.png}"

mkdir -p "$OUT"
python3 - "$MASTER" "$OUT" <<'PY'
import os
import sys

from PIL import Image

master, out = sys.argv[1:3]
im = Image.open(master).convert("RGBA")
if im.width != im.height or im.width < 1024:
    sys.exit(f"error: the master must be square and at least 1024 px, not {im.width}x{im.height}")
if im.width != 1024:
    im = im.resize((1024, 1024), Image.Resampling.LANCZOS)
# Pillow takes the sizes to embed from the largest picture it is given.
im.save(
    os.path.join(out, "circuito.ico"),
    sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)],
)
im.save(os.path.join(out, "circuito.icns"), format="ICNS")
PY
find "$OUT" -type f | sort
