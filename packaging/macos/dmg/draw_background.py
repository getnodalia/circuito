#!/usr/bin/env python3
"""Draw the Finder window background of Circuito's DMG, as background.svg.

    pip install fonttools
    NODALIA_DESIGN_DIR=/path/to/nodalia-design python3 packaging/macos/dmg/draw_background.py

The layout, the lettering and the arrow's place are nodalia-design's own
(tools/dmg-background/draw_background.py): this file only adds Circuito to its table, with a
picture of what the app does (an RC low-pass drawn as a schematic, and its step response) and
Circuito's accent, #C99A06. When nodalia-design carries a Circuito entry of its own, delete this
file and regenerate with that one. The output is flat inks and outlined text, byte-for-byte
reproducible, and needs no fonts to render.
"""
import importlib.util
import math
import os
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
DESIGN = Path(os.environ.get("NODALIA_DESIGN_DIR", HERE.parents[3] / "nodalia-design"))
SOURCE = DESIGN / "tools" / "dmg-background" / "draw_background.py"
if not SOURCE.is_file():
    sys.exit(f"error: {SOURCE} not found; set NODALIA_DESIGN_DIR to a nodalia-design checkout")
spec = importlib.util.spec_from_file_location("ndl_dmg_background", SOURCE)
ndl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ndl)

WHITE = "#FFFFFF"


def m_circuito(c, d):
    o = [f'<rect width="{ndl.AW}" height="{ndl.AH}" fill="{c}"/>']
    ink = ndl.INK
    # An RC low-pass as a schematic: input, resistor, the node, a capacitor to ground, output.
    y = 96
    stroke = f'fill="none" stroke="{WHITE}" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"'
    zig = [(62, y), (68, y), (74, y - 16), (84, y + 16), (94, y - 16), (104, y + 16), (110, y), (116, y)]
    path = "M" + " L".join(f"{x},{yy}" for x, yy in zig)
    o.append(f'<path d="M33,{y} L62,{y}" {stroke}/>')
    o.append(f'<path d="{path}" {stroke}/>')
    o.append(f'<path d="M116,{y} L191,{y}" {stroke}/>')
    o.append(f'<circle cx="28" cy="{y}" r="5" fill="{c}" stroke="{WHITE}" stroke-width="3"/>')
    o.append(f'<circle cx="196" cy="{y}" r="5" fill="{c}" stroke="{WHITE}" stroke-width="3"/>')
    o.append(f'<circle cx="140" cy="{y}" r="4.5" fill="{ink}"/>')
    o.append(f'<path d="M140,{y} L140,{y + 28} M124,{y + 28} L156,{y + 28} M124,{y + 38} L156,{y + 38} M140,{y + 38} L140,{y + 58}" {stroke}/>')
    gy = y + 58
    o.append(f'<path d="M122,{gy} L158,{gy} M129,{gy + 7} L151,{gy + 7} M136,{gy + 14} L144,{gy + 14}" {stroke}/>')
    # Its step response in a plot: the input, the curve, and one time constant marked.
    px, py, pw, ph = 16, 206, 192, 160
    o.append(f'<rect x="{px}" y="{py}" width="{pw}" height="{ph}" rx="10" fill="{d}"/>')
    x0, y0, x1, ytop = 36, 342, 194, 232
    for gy2 in (262, 302):
        o.append(f'<line x1="{x0}" y1="{gy2}" x2="{x1}" y2="{gy2}" stroke="{WHITE}" stroke-width="1" opacity=".25"/>')
    o.append(f'<path d="M{x0},{ytop - 8} L{x0},{y0} L{x1},{y0}" fill="none" stroke="{WHITE}" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>')
    level = y0 - 92
    o.append(f'<line x1="{x0}" y1="{level}" x2="{x1}" y2="{level}" stroke="{WHITE}" stroke-width="2" stroke-dasharray="2 5" stroke-linecap="round" opacity=".8"/>')
    span = x1 - 8 - x0
    pts = []
    for i in range(0, 61):
        t = i / 60 * 5.0
        pts.append(f"{x0 + t / 5.0 * span:.1f},{y0 - 92 * (1 - math.exp(-t)):.1f}")
    o.append(f'<path d="M{x0},{y0} L' + " L".join(pts) + f'" fill="none" stroke="{WHITE}" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"/>')
    tx = x0 + 1.0 / 5.0 * span
    ty = y0 - 92 * (1 - math.exp(-1.0))
    o.append(f'<path d="M{tx:.1f},{y0} L{tx:.1f},{ty:.1f}" stroke="{ink}" stroke-width="2" stroke-dasharray="3 3" fill="none"/>')
    o.append(f'<circle cx="{tx:.1f}" cy="{ty:.1f}" r="5" fill="{ink}" stroke="{WHITE}" stroke-width="2"/>')
    return "".join(o)


def a_circuito(c):
    al, ar, yc, ink = ndl.AL, ndl.AR, ndl.AYc, ndl.INK
    zig = [(al + 18, yc), (al + 22, yc), (al + 27, yc - 8), (al + 36, yc + 8), (al + 45, yc - 8), (al + 54, yc + 8), (al + 59, yc), (al + 63, yc)]
    path = "M" + " L".join(f"{x},{yy}" for x, yy in zig)
    return (f'<line x1="{al}" y1="{yc}" x2="{al + 18}" y2="{yc}" stroke="{ink}" stroke-width="1.6" stroke-linecap="round"/>'
            f'<path d="{path}" fill="none" stroke="{c}" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"/>'
            f'<line x1="{al + 63}" y1="{yc}" x2="{ar - 12}" y2="{yc}" stroke="{ink}" stroke-width="1.6" stroke-linecap="round"/>'
            + ndl.arrowhead(ar, yc, ink))


ndl.APPS["circuito"] = ("Circuito", "Electrónica y simulación", "#C99A06", "#A07A04", m_circuito, a_circuito)

if __name__ == "__main__":
    drawn = ndl.svg("circuito").replace(
        "Drawn by tools/dmg-background/draw_background.py in nodalia-design: change it there and regenerate.",
        "Drawn by packaging/macos/dmg/draw_background.py, on nodalia-design's DMG layout: change it there and regenerate.",
    )
    (HERE / "background.svg").write_text(drawn)
