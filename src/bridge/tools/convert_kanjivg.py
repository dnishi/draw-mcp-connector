"""Converts KanjiVG glyphs into the point list data (glyphs.bin) the bridge uses to write text.

KanjiVG (https://kanjivg.tagaini.net) is glyph data by Ulrich Apel under Creative Commons
Attribution-Share Alike 3.0. The converted data is distributed under the same license
(assets/kanjivg/README.md).

Usage (needs only the standard library):
    python3 -I tools/convert_kanjivg.py <kanjivg-YYYYMMDD-main.zip> assets/kanjivg/glyphs.bin

A glyph is a list of strokes inside a 109×109 frame (y points down). Curves become polylines, and coordinates are rounded to 0.5 units.

Format of glyphs.bin (all integers are little-endian):
    u32 character count N
    N pairs of (u32 code point, u32 data offset), sorted by code point
    Data per character: u8 stroke count, then per stroke (u16 point count, point count × (u8 x, u8 y)). Coordinates are doubled
"""

import math
import re
import struct
import sys
import zipfile

UNITS_PER_STEP = 2  # Coordinates are kept in 0.5 units
# Maximum deviation from the curve when turning it into a polyline (frame units). 0.25px at a text size of 109px
FLATTEN_TOLERANCE = 0.25
MAX_SEGMENTS_PER_CURVE = 32

# Symbols missing from KanjiVG, added here in the same 109×109 frame (SVG paths using only M, L, C)
EXTRA_GLYPHS = {
    '"': ["M42,16 L40,36", "M64,16 L62,36"],
    "#": ["M44,20 L36,92", "M70,20 L62,92", "M26,44 L84,44", "M22,70 L80,70"],
    "$": [
        "M76,32 C70,20 38,18 34,34 C30,50 76,52 76,72 C76,92 40,94 30,80",
        "M54,12 L54,100",
    ],
    "%": [
        "M36,22 C26,22 24,46 36,46 C48,46 46,22 36,22",
        "M74,62 C64,62 62,86 74,86 C86,86 84,62 74,62",
        "M80,20 L30,90",
    ],
    "&": ["M84,92 L42,46 C30,32 40,16 52,18 C66,20 66,36 50,48 L38,58 C22,72 30,92 50,92 C64,92 74,80 80,64"],
    "'": ["M54,16 L52,36"],
    "(": ["M64,14 C40,36 40,76 64,98"],
    ")": ["M44,14 C68,36 68,76 44,98"],
    "*": ["M54,24 L54,64", "M36,32 L72,56", "M72,32 L36,56"],
    "+": ["M54,34 L54,82", "M30,58 L78,58"],
    "-": ["M34,58 L74,58"],
    "/": ["M76,14 L32,98"],
    "<": ["M78,30 L30,58 L78,86"],
    "=": ["M30,46 L78,46", "M30,70 L78,70"],
    ">": ["M30,30 L78,58 L30,86"],
    "@": [
        "M68,44 C60,34 42,40 42,58 C42,74 62,72 68,58 L70,44 L70,66 C70,78 90,74 90,56 C90,30 70,18 54,18 C30,18 18,38 18,56 C18,80 36,94 56,94 C66,94 74,90 80,86"
    ],
    "[": ["M62,14 L46,14 L46,98 L62,98"],
    "\\": ["M32,14 L76,98"],
    "]": ["M46,14 L62,14 L62,98 L46,98"],
    "^": ["M36,40 L54,18 L72,40"],
    "_": ["M26,98 L82,98"],
    "`": ["M46,16 L60,30"],
    "{": ["M64,14 C48,14 52,30 52,44 C52,52 48,56 40,56 C48,56 52,60 52,68 C52,82 48,98 64,98"],
    "|": ["M54,10 L54,102"],
    "}": ["M44,14 C60,14 56,30 56,44 C56,52 60,56 68,56 C60,56 56,60 56,68 C56,82 60,98 44,98"],
    "~": ["M30,62 C36,48 48,50 54,56 C60,62 72,64 78,50"],
    "「": ["M44,18 L28,18 L28,62"],
    "」": ["M64,48 L64,92 L80,92"],
    "（": ["M74,10 C46,34 46,76 74,100"],
    "）": ["M34,10 C62,34 62,76 34,100"],
}

NUMBER = re.compile(r"[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?")
TOKEN = re.compile(r"[MmLlCcSs]|" + NUMBER.pattern)


def cubic(p0, p1, p2, p3):
    """Turns a cubic Bézier curve into a polyline (excluding the start point).

    The number of segments follows Wang's formula so the deviation from the curve stays within FLATTEN_TOLERANCE
    (nearly straight strokes need few points)
    """
    bend = max(
        math.hypot(*(a - 2 * b + c for a, b, c in zip(p0, p1, p2))),
        math.hypot(*(a - 2 * b + c for a, b, c in zip(p1, p2, p3))),
    )
    n = max(1, min(MAX_SEGMENTS_PER_CURVE, math.ceil(math.sqrt(0.75 * bend / FLATTEN_TOLERANCE))))
    points = []
    for i in range(1, n + 1):
        t = i / n
        u = 1 - t
        points.append(
            tuple(
                u * u * u * a + 3 * u * u * t * b + 3 * u * t * t * c + t * t * t * d
                for a, b, c, d in zip(p0, p1, p2, p3)
            )
        )
    return points


def parse_path(d):
    """Turns an SVG path (M, L, C, S and lowercase) into a list of polylines. Each M starts a new polyline"""
    tokens = TOKEN.findall(d)
    polylines = []
    command = None
    current = (0.0, 0.0)
    last_control = None  # Second control point of the previous C or S, mirrored by S
    i = 0

    def take(count):
        nonlocal i
        values = [float(v) for v in tokens[i : i + count]]
        if len(values) != count:
            raise ValueError(f"パスの数が足りない: {d}")
        i += count
        return values

    while i < len(tokens):
        if tokens[i].isalpha():
            command = tokens[i]
            i += 1
        if command is None:
            raise ValueError(f"パスがコマンドで始まらない: {d}")
        relative = command.islower()
        offset = current if relative else (0.0, 0.0)

        def point(x, y):
            return (offset[0] + x, offset[1] + y)

        kind = command.upper()
        if kind == "M":
            current = point(*take(2))
            polylines.append([current])
            last_control = None
            # Coordinate pairs following M are treated as L (per the SVG spec)
            command = "l" if relative else "L"
        elif kind == "L":
            current = point(*take(2))
            polylines[-1].append(current)
            last_control = None
        elif kind == "C":
            x1, y1, x2, y2, x, y = take(6)
            c1, c2, end = point(x1, y1), point(x2, y2), point(x, y)
            polylines[-1].extend(cubic(current, c1, c2, end))
            current, last_control = end, c2
        elif kind == "S":
            x2, y2, x, y = take(4)
            c1 = (
                (2 * current[0] - last_control[0], 2 * current[1] - last_control[1])
                if last_control
                else current
            )
            c2, end = point(x2, y2), point(x, y)
            polylines[-1].extend(cubic(current, c1, c2, end))
            current, last_control = end, c2
    return polylines


def quantize(polyline):
    """Rounds to 0.5 units and drops consecutive points that became identical"""
    points = []
    for x, y in polyline:
        q = tuple(min(255, max(0, round(v * UNITS_PER_STEP))) for v in (x, y))
        if not points or points[-1] != q:
            points.append(q)
    # Even a single point (a tiny dot) becomes 2 points so it can be drawn as a line
    return points * 2 if len(points) == 1 else points


def encode_glyph(paths):
    strokes = [quantize(line) for d in paths for line in parse_path(d)]
    data = bytearray([len(strokes)])
    for points in strokes:
        data += struct.pack("<H", len(points))
        for x, y in points:
            data += bytes([x, y])
    return bytes(data)


def read_kanjivg(zip_path):
    """Code point → list of paths. Variants (file names with -) are excluded"""
    glyphs = {}
    with zipfile.ZipFile(zip_path) as archive:
        for name in archive.namelist():
            match = re.fullmatch(r"kanji/([0-9a-f]{5})\.svg", name)
            if match:
                svg = archive.read(name).decode("utf-8")
                glyphs[int(match.group(1), 16)] = re.findall(r'<path [^>]*\bd="([^"]+)"', svg)
    return glyphs


def main(zip_path, out_path):
    glyphs = read_kanjivg(zip_path)
    for char, paths in EXTRA_GLYPHS.items():
        if ord(char) in glyphs:
            raise ValueError(f"KanjiVG にある字を足そうとしている: {char}")
        glyphs[ord(char)] = paths

    codepoints = sorted(glyphs)
    bodies = [encode_glyph(glyphs[c]) for c in codepoints]
    index_size = 4 + 8 * len(codepoints)
    out = bytearray(struct.pack("<I", len(codepoints)))
    position = index_size
    for codepoint, body in zip(codepoints, bodies):
        out += struct.pack("<II", codepoint, position)
        position += len(body)
    for body in bodies:
        out += body
    with open(out_path, "wb") as f:
        f.write(out)
    print(f"{len(codepoints)} 字、{len(out)} バイト")


if __name__ == "__main__":
    main(*sys.argv[1:])
