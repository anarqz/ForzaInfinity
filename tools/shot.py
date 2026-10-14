"""Screenshot of the primary screen to a PNG (no PIL): python shot.py out.png [scale_divisor=2]"""
import ctypes, ctypes.wintypes as wt, struct, sys, zlib

u32, g32 = ctypes.WinDLL("user32"), ctypes.WinDLL("gdi32")
u32.SetProcessDPIAware()


def grab(div=2):
    w, h = u32.GetSystemMetrics(0), u32.GetSystemMetrics(1)
    sdc = u32.GetDC(0)
    mdc = g32.CreateCompatibleDC(sdc)
    bmp = g32.CreateCompatibleBitmap(sdc, w // div, h // div)
    g32.SelectObject(mdc, bmp)
    g32.SetStretchBltMode(mdc, 4)  # HALFTONE
    g32.StretchBlt(mdc, 0, 0, w // div, h // div, sdc, 0, 0, w, h, 0x00CC0020 | 0x40000000)
    W, H = w // div, h // div

    class BIH(ctypes.Structure):
        _fields_ = [("size", wt.DWORD), ("w", wt.LONG), ("h", wt.LONG), ("planes", wt.WORD), ("bits", wt.WORD),
                    ("comp", wt.DWORD), ("img", wt.DWORD), ("xp", wt.LONG), ("yp", wt.LONG), ("cu", wt.DWORD), ("ci", wt.DWORD)]
    bi = BIH(ctypes.sizeof(BIH), W, -H, 1, 32, 0, 0, 0, 0, 0, 0)
    buf = ctypes.create_string_buffer(W * H * 4)
    g32.GetDIBits(mdc, bmp, 0, H, buf, ctypes.byref(bi), 0)
    g32.DeleteObject(bmp); g32.DeleteDC(mdc); u32.ReleaseDC(0, sdc)
    return W, H, buf.raw


def png(path, W, H, bgra):
    rows = bytearray()
    for y in range(H):
        row = bgra[y * W * 4:(y + 1) * W * 4]
        rgb = bytearray(W * 3)
        rgb[0::3], rgb[1::3], rgb[2::3] = row[2::4], row[1::4], row[0::4]
        rows += b"\0" + rgb
    def chunk(t, d):
        return struct.pack(">I", len(d)) + t + d + struct.pack(">I", zlib.crc32(t + d) & 0xffffffff)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(bytes(rows), 6)) + chunk(b"IEND", b""))


if __name__ == "__main__":
    W, H, data = grab(int(sys.argv[2]) if len(sys.argv) > 2 else 2)
    png(sys.argv[1], W, H, data)
    print(sys.argv[1], W, H)
