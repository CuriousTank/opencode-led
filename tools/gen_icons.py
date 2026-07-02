"""Generate red/yellow/green circular traffic-light PNGs (96x96, transparent background).

Pure standard library (zlib + struct), no third-party deps.
Draws a radial-shaded circle with a darker rim and a soft glow halo.
"""
import math
import os
import struct
import zlib

SIZE = 96
R = 42  # circle radius
CX = CY = SIZE // 2


def write_png(path, width, height, rgba_bytes):
    def chunk(tag, data):
        return (
            struct.pack(">I", len(data))
            + tag
            + data
            + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)
        )

    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0)  # 8-bit RGBA
    raw = bytearray()
    stride = width * 4
    for y in range(height):
        raw.append(0)  # filter type 0
        raw.extend(rgba_bytes[y * stride : (y + 1) * stride])
    idat = zlib.compress(bytes(raw), 9)
    with open(path, "wb") as f:
        f.write(sig + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b""))


def shade(rgb_core, rgb_mid, rgb_edge):
    """Return per-pixel RGBA for one light with given core/mid/edge colors."""
    px = bytearray(SIZE * SIZE * 4)
    for y in range(SIZE):
        for x in range(SIZE):
            dx = x - CX
            dy = y - CY
            d = math.sqrt(dx * dx + dy * dy)
            i = (y * SIZE + x) * 4
            if d <= R - 2:
                t = d / (R - 2)
                if t < 0.55:
                    u = t / 0.55
                    r = rgb_core[0] * (1 - u) + rgb_mid[0] * u
                    g = rgb_core[1] * (1 - u) + rgb_mid[1] * u
                    b = rgb_core[2] * (1 - u) + rgb_mid[2] * u
                else:
                    u = (t - 0.55) / 0.45
                    r = rgb_mid[0] * (1 - u) + rgb_edge[0] * u
                    g = rgb_mid[1] * (1 - u) + rgb_edge[1] * u
                    b = rgb_mid[2] * (1 - u) + rgb_edge[2] * u
                px[i : i + 3] = bytes((int(r), int(g), int(b)))
                px[i + 3] = 255
            elif d <= R:
                u = (d - (R - 2)) / 2.0
                r = rgb_edge[0] * (1 - u)
                g = rgb_edge[1] * (1 - u)
                b = rgb_edge[2] * (1 - u)
                px[i : i + 3] = bytes((int(r), int(g), int(b)))
                px[i + 3] = 255
            elif d <= R + 8:
                u = (d - R) / 8.0
                a = int(90 * (1 - u))
                px[i : i + 3] = bytes(rgb_mid)
                px[i + 3] = a
    return px


def main():
    out = os.path.abspath(
        os.path.join(os.path.dirname(__file__), "..", "assets")
    )
    os.makedirs(out, exist_ok=True)
    lights = {
        "red.png": ((255, 120, 90), (230, 60, 50), (150, 25, 20)),
        "yellow.png": ((255, 230, 130), (245, 195, 60), (175, 135, 25)),
        "green.png": ((150, 245, 150), (70, 210, 95), (30, 140, 55)),
    }
    for name, (core, mid, edge) in lights.items():
        write_png(os.path.join(out, name), SIZE, SIZE, shade(core, mid, edge))
        print("wrote", os.path.join(out, name))


if __name__ == "__main__":
    main()
