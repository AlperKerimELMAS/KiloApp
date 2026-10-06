"""Builds AppIcon.icns from icon.svg: renders with Quick Look, cuts the
squircle out with an anti-aliased alpha mask (Quick Look adds a white
background), then lets sips/iconutil make every size."""
import math, os, shutil, struct, subprocess, sys, tempfile, zlib

here = os.path.dirname(os.path.abspath(__file__))
tmp = tempfile.mkdtemp()
subprocess.run(["qlmanage", "-t", "-s", "1024", "-o", tmp, os.path.join(here, "icon.svg")], check=True, capture_output=True)
png = open(os.path.join(tmp, "icon.svg.png"), "rb").read()

# --- minimal PNG decode (8-bit RGB/RGBA, non-interlaced) ---
pos, idat = 8, b""
while pos < len(png):
    n, kind = struct.unpack(">I4s", png[pos:pos + 8]); data = png[pos + 8:pos + 8 + n]; pos += 12 + n
    if kind == b"IHDR": w, h, depth, ctype = struct.unpack(">IIBB", data[:10])
    elif kind == b"IDAT": idat += data
assert depth == 8 and ctype in (2, 6), (depth, ctype)
bpp = 4 if ctype == 6 else 3
raw, stride, rows, prev = zlib.decompress(idat), w * bpp, [], bytearray(w * bpp)
for y in range(h):
    f, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
    for i in range(stride):
        a = line[i - bpp] if i >= bpp else 0; b = prev[i]; c = prev[i - bpp] if i >= bpp else 0
        if f == 1: line[i] = (line[i] + a) & 255
        elif f == 2: line[i] = (line[i] + b) & 255
        elif f == 3: line[i] = (line[i] + (a + b) // 2) & 255
        elif f == 4:
            p = a + b - c; pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
            line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
    rows.append(line); prev = line

# --- alpha = coverage of the rounded rect (100..924, r=186), 4x4 supersampled at edges ---
x0, y0, x1, y1, r = 100, 100, 924, 924, 186
def inside(px, py):
    cx = min(max(px, x0 + r), x1 - r); cy = min(max(py, y0 + r), y1 - r)
    return x0 <= px <= x1 and y0 <= py <= y1 and (px - cx) ** 2 + (py - cy) ** 2 <= r * r
out = bytearray()
for y in range(h):
    out.append(0)
    for x in range(w):
        px = rows[y][x * bpp:x * bpp + 3]
        corners = [inside(x, y), inside(x + 1, y), inside(x, y + 1), inside(x + 1, y + 1)]
        if all(corners): a = 255
        elif not any(corners) and not inside(x + 0.5, y + 0.5): a = 0
        else: a = round(255 * sum(inside(x + (i + .5) / 4, y + (j + .5) / 4) for i in range(4) for j in range(4)) / 16)
        out += bytes(px) + bytes([a])
def chunk(kind, data): return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
master = os.path.join(tmp, "icon_1024.png")
open(master, "wb").write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(bytes(out), 9)) + chunk(b"IEND", b""))

iconset = os.path.join(tmp, "AppIcon.iconset"); os.mkdir(iconset)
for size in (16, 32, 128, 256, 512):
    for scale in (1, 2):
        px = size * scale; name = f"icon_{size}x{size}{'@2x' if scale == 2 else ''}.png"
        subprocess.run(["sips", "-z", str(px), str(px), master, "--out", os.path.join(iconset, name)], check=True, capture_output=True)
subprocess.run(["iconutil", "-c", "icns", iconset, "-o", os.path.join(here, "AppIcon.icns")], check=True)
shutil.copy(master, os.path.join(sys.argv[1] if len(sys.argv) > 1 else tmp, "icon_preview.png"))
print("AppIcon.icns:", os.path.getsize(os.path.join(here, "AppIcon.icns")), "bytes")
