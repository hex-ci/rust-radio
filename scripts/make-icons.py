#!/usr/bin/env python3
"""从 app-icon-1024.png 生成整套平台图标（只用标准库，不需要 Pillow）。

产物（`crates/rust-radio-app/assets/` 下）：
  icons/16x16.png 32x32.png 64x64.png 128x128.png 256x256.png 512x512.png 512x512@2x.png
      给 cargo-packager：macOS 由这组图生成 icns，Linux 按尺寸装进 hicolor/<N>x<N>/apps/
      （icns 的槽位是「像素尺寸 ÷ 密度」，所以只出这些合法尺寸；512x512@2x 是 1024px）
  icons/app.ico
      给 build.rs 编进 exe：任务栏取窗口的 ICON_BIG，winit 只设 ICON_SMALL，
      窗口类 `hIcon: 0`，所以任务栏/资源管理器读的是 exe 内嵌的图标资源。
      .ico 内部 16/24/32/48/64/128 存 32 位 DIB，256 存 PNG。

缩小用面积平均，先按 alpha 预乘再平均，避免透明边缘渗出深色。

用法：
    python scripts/make-icons.py
    python scripts/make-icons.py 源.png 输出目录
"""

from __future__ import annotations

import struct
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SRC = ROOT / "crates/rust-radio-app/assets/app-icon-1024.png"
DEFAULT_OUT = ROOT / "crates/rust-radio-app/assets/icons"

# packager 需要的一组尺寸；1024 只以 @2x 形式出现（icns 的 512@2x 槽）
PNG_SIZES = (16, 32, 64, 128, 256, 512)
RETINA_SIZE = 1024
DIB_SIZES = (16, 24, 32, 48, 64, 128)
ICO_PNG_SIZE = 256


def read_png(path: Path) -> tuple[int, int, list[bytearray]]:
    """解 8 bit RGBA、非隔行的 PNG，返回 (宽, 高, 每行 RGBA 字节)。"""
    data = path.read_bytes()
    if data[:8] != b"\x89PNG\r\n\x1a\n":
        sys.exit(f"{path} 不是 PNG")

    pos = 8
    width = height = 0
    idat = bytearray()
    while pos < len(data):
        length = struct.unpack(">I", data[pos : pos + 4])[0]
        kind = data[pos + 4 : pos + 8]
        body = data[pos + 8 : pos + 8 + length]
        if kind == b"IHDR":
            width, height, depth, color, _, _, interlace = struct.unpack(">IIBBBBB", body)
            if (depth, color, interlace) != (8, 6, 0):
                sys.exit(
                    f"只支持 8bit RGBA 非隔行 PNG，当前 depth={depth} color={color} interlace={interlace}"
                )
        elif kind == b"IDAT":
            idat += body
        elif kind == b"IEND":
            break
        pos += 12 + length

    if not width or not height:
        sys.exit("PNG 缺少 IHDR")

    raw = zlib.decompress(bytes(idat))
    stride = width * 4
    rows: list[bytearray] = []
    prev = bytearray(stride)
    off = 0
    for _ in range(height):
        filter_type = raw[off]
        line = bytearray(raw[off + 1 : off + 1 + stride])
        off += 1 + stride
        if filter_type == 1:  # Sub
            for i in range(4, stride):
                line[i] = (line[i] + line[i - 4]) & 0xFF
        elif filter_type == 2:  # Up
            for i in range(stride):
                line[i] = (line[i] + prev[i]) & 0xFF
        elif filter_type == 3:  # Average
            for i in range(stride):
                left = line[i - 4] if i >= 4 else 0
                line[i] = (line[i] + ((left + prev[i]) >> 1)) & 0xFF
        elif filter_type == 4:  # Paeth
            for i in range(stride):
                a = line[i - 4] if i >= 4 else 0
                b = prev[i]
                c = prev[i - 4] if i >= 4 else 0
                p = a + b - c
                pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
                pred = a if (pa <= pb and pa <= pc) else (b if pb <= pc else c)
                line[i] = (line[i] + pred) & 0xFF
        elif filter_type != 0:
            sys.exit(f"未知的 PNG 滤波类型 {filter_type}")
        rows.append(line)
        prev = line
    return width, height, rows


def box_resize(width: int, height: int, rows: list[bytearray], size: int) -> list[bytearray]:
    """面积平均缩放到 size×size，输出 RGBA（非预乘）。"""
    scale = width / size
    out: list[bytearray] = []
    for ty in range(size):
        y0, y1 = ty * scale, (ty + 1) * scale
        weights_y = []
        for sy in range(max(0, int(y0)), min(height, int(y1) + 1)):
            cover = min(y1, sy + 1) - max(y0, sy)
            if cover > 0:
                weights_y.append((sy, cover))
        total = sum(w for _, w in weights_y)

        acc = [[0.0, 0.0, 0.0, 0.0] for _ in range(size)]
        for sy, wy in weights_y:
            src = rows[sy]
            for tx in range(size):
                x0, x1 = tx * scale, (tx + 1) * scale
                r = g = b = a = 0.0
                for sx in range(max(0, int(x0)), min(width, int(x1) + 1)):
                    cover = min(x1, sx + 1) - max(x0, sx)
                    if cover <= 0:
                        continue
                    i = sx * 4
                    alpha = src[i + 3] / 255.0
                    r += src[i] * alpha * cover
                    g += src[i + 1] * alpha * cover
                    b += src[i + 2] * alpha * cover
                    a += alpha * cover
                acc[tx][0] += r * wy
                acc[tx][1] += g * wy
                acc[tx][2] += b * wy
                acc[tx][3] += a * wy

        line = bytearray(size * 4)
        for tx in range(size):
            r, g, b, a = (v / total for v in acc[tx])
            if a > 1e-6:  # 反预乘
                r, g, b = r / a, g / a, b / a
            line[tx * 4 : tx * 4 + 4] = bytes(
                (
                    min(255, round(r)),
                    min(255, round(g)),
                    min(255, round(b)),
                    min(255, round(a * 255)),
                )
            )
        out.append(line)
    return out


def dib_bytes(rows: list[bytearray], size: int) -> bytes:
    """ICO 里的 DIB 条目：BITMAPINFOHEADER + 自下而上的 BGRA + AND 掩码。"""
    body = bytearray()
    for y in range(size - 1, -1, -1):
        src = rows[y]
        for x in range(size):
            i = x * 4
            body += bytes((src[i + 2], src[i + 1], src[i], src[i + 3]))

    # 透明交给 alpha，AND 掩码全 0；每行按 4 字节对齐
    mask_stride = ((size + 31) // 32) * 4
    header = struct.pack(
        "<IiiHHIIiiII",
        40,  # biSize
        size,
        size * 2,  # XOR + AND 两张图叠在一起
        1,  # biPlanes
        32,  # biBitCount
        0,  # biCompression = BI_RGB
        len(body),
        0,
        0,
        0,
        0,
    )
    return header + bytes(body) + b"\x00" * (mask_stride * size)


def png_bytes(rows: list[bytearray], size: int) -> bytes:
    """把 RGBA 行编成最小 PNG（逐行 filter 0 + zlib）。"""
    raw = bytearray()
    for row in rows:
        raw.append(0)
        raw += row

    def chunk(kind: bytes, body: bytes) -> bytes:
        return (
            struct.pack(">I", len(body))
            + kind
            + body
            + struct.pack(">I", zlib.crc32(kind + body) & 0xFFFFFFFF)
        )

    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )

def build_all(src: Path, out_dir: Path) -> None:
    width, height, rows = read_png(src)
    if width != height:
        sys.exit(f"{src} 不是正方形（{width}x{height}），无法生成图标")

    out_dir.mkdir(parents=True, exist_ok=True)
    cache = {}

    def resized(size: int) -> list[bytearray]:
        if size not in cache:
            cache[size] = box_resize(width, height, rows, size)
        return cache[size]

    for size in PNG_SIZES:
        path = out_dir / f"{size}x{size}.png"
        path.write_bytes(png_bytes(resized(size), size))
        print(f"==> {path.name}")

    retina = out_dir / "512x512@2x.png"
    retina.write_bytes(png_bytes(resized(RETINA_SIZE), RETINA_SIZE))
    print(f"==> {retina.name}（{RETINA_SIZE}px）")

    entries = [(size, dib_bytes(resized(size), size)) for size in DIB_SIZES]
    entries.append((ICO_PNG_SIZE, png_bytes(resized(ICO_PNG_SIZE), ICO_PNG_SIZE)))
    offset = 6 + 16 * len(entries)
    directory = bytearray()
    payload = bytearray()
    for size, data in entries:
        directory += struct.pack(
            "<BBBBHHII",
            size if size < 256 else 0,
            size if size < 256 else 0,
            0,
            0,
            1,
            32,
            len(data),
            offset,
        )
        payload += data
        offset += len(data)
    ico = out_dir / "app.ico"
    ico.write_bytes(struct.pack("<HHH", 0, 1, len(entries)) + bytes(directory) + bytes(payload))
    print(f"==> {ico.name}（{len(entries)} 档，{ico.stat().st_size} 字节）")


def main() -> None:
    src = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_SRC
    out_dir = Path(sys.argv[2]) if len(sys.argv) > 2 else DEFAULT_OUT
    if not src.exists():
        sys.exit(f"找不到源图：{src}")
    build_all(src, out_dir)


if __name__ == "__main__":
    main()
