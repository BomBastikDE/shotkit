"""Erzeugt src/shotkit.ico (PNG-basiert, mehrere Größen) ohne externe Abhängigkeiten."""
import math, struct, zlib, pathlib

def render(n):
    ss = 4  # Supersampling
    N = n * ss
    def inside(x, y):
        # Koordinaten im 64er-Raster
        u, v = x * 64 / N, y * 64 / N
        # abgerundetes Quadrat 2..62, Radius 14
        r = 14
        cx = min(max(u, 2 + r), 62 - r); cy = min(max(v, 2 + r), 62 - r)
        if (u - cx) ** 2 + (v - cy) ** 2 > r * r or not (2 <= u <= 62 and 2 <= v <= 62):
            return None
        t = v / 64
        base = (int(255 + (214 - 255) * t), int(92 + (36 - 92) * t), int(72 + (58 - 72) * t))
        def seg(ax, ay, bx, by, w=3.0):
            dx, dy = bx - ax, by - ay
            k = max(0, min(1, ((u - ax) * dx + (v - ay) * dy) / (dx * dx + dy * dy)))
            return math.hypot(u - (ax + k * dx), v - (ay + k * dy)) <= w
        lines = [(14,26,14,14),(14,14,26,14),(38,14,50,14),(50,14,50,26),
                 (50,38,50,50),(50,50,38,50),(26,50,14,50),(14,50,14,38)]
        if any(seg(*l) for l in lines) or math.hypot(u - 32, v - 32) <= 7:
            return (255, 255, 255)
        return base
    rows = []
    for y in range(n):
        row = bytearray([0])
        for x in range(n):
            acc = [0, 0, 0, 0]
            for sy in range(ss):
                for sx in range(ss):
                    c = inside(x * ss + sx + 0.5, y * ss + sy + 0.5)
                    if c:
                        acc[0] += c[0]; acc[1] += c[1]; acc[2] += c[2]; acc[3] += 1
            a = acc[3]
            row += bytes([acc[0] // a, acc[1] // a, acc[2] // a, a * 255 // (ss * ss)]) if a else bytes(4)
        rows.append(bytes(row))
    raw = b"".join(rows)
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", n, n, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 9)) + chunk(b"IEND", b""))

sizes = [16, 24, 32, 48, 64, 256]
pngs = [render(s) for s in sizes]
out = struct.pack("<HHH", 0, 1, len(sizes))
offset = 6 + 16 * len(sizes)
for s, p in zip(sizes, pngs):
    out += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(p), offset)
    offset += len(p)
out += b"".join(pngs)
target = pathlib.Path(__file__).resolve().parent.parent / "src" / "shotkit.ico"
target.write_bytes(out)
print(target, len(out), "bytes")
