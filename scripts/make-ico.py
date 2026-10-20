#!/usr/bin/env python3
"""从 app-icon-1024.png 生成 Windows 用的 app.ico（只用标准库，不需要 Pillow）。

为什么要单独出一份 .ico：任务栏按钮取窗口的 ICON_BIG，winit 只设了 ICON_SMALL
（egui 的 `ViewportBuilder::with_icon` 走的也是这条），窗口类又是 `hIcon: 0`，
所以任务栏/资源管理器实际用的是 exe 里内嵌的图标资源——由 build.rs 通过
winresource 把这份 .ico 编进去。

尺寸：16/24/32/48/64/128 存 32 位 DIB（兼容 rc.exe 与老工具），256 存 PNG。
缩小用面积平均，先按 alpha 预乘再平均，避免透明边缘渗出深色。

用法：
    python scripts/make-ico.py                     # 用默认路径
    python scripts/make-ico.py 源.png 目标.ico      # 指定输入输出
"""

from __future__ import annotations

import struct
import sys
import zlib
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SRC = ROOT / "crates/rust-radio-app/assets/app-icon-1024.png"
DEFAULT_DST = ROOT / "crates/rust-radio-app/assets/app.ico"

DIB_SIZES = (16, 24, 32, 48, 64, 128)
PNG_SIZE = 256


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


def build_ico(src: Path, dst: Path) -> None:
    width, height, rows = read_png(src)
    if width != height:
        sys.exit(f"{src} 不是正方形（{width}x{height}），无法生成图标")

    entries: list[tuple[int, bytes]] = []
    for size in DIB_SIZES:
        entries.append((size, dib_bytes(box_resize(width, height, rows, size), size)))
    entries.append((PNG_SIZE, png_bytes(box_resize(width, height, rows, PNG_SIZE), PNG_SIZE)))

    offset = 6 + 16 * len(entries)
    directory = bytearray()
    payload = bytearray()
    for size, data in entries:
        # ICO 的宽高字段是 1 字节，256 记作 0
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

    dst.write_bytes(struct.pack("<HHH", 0, 1, len(entries)) + bytes(directory) + bytes(payload))
    print(f"==> {dst}（{len(entries)} 档，{dst.stat().st_size} 字节）")


def main() -> None:
    src = Path(sys.argv[1]) if len(sys.argv) > 1 else DEFAULT_SRC
    dst = Path(sys.argv[2]) if len(sys.argv) > 2 else DEFAULT_DST
    if not src.exists():
        sys.exit(f"找不到源图：{src}")
    build_ico(src, dst)


if __name__ == "__main__":
    main()
