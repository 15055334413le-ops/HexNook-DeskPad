# 开发自检：把 DeskPad --render-test 输出的 32 位 BMP 转成 PNG（只用标准库）
import struct, sys, zlib
def conv(src, dst, scale=1):
    b = open(src, 'rb').read()
    off = struct.unpack_from('<I', b, 10)[0]
    w, h = struct.unpack_from('<ii', b, 18)
    top_down = h < 0; h = abs(h)
    rows = []
    for y in range(0, h, scale):
        yy = y if top_down else h - 1 - y
        row = b[off + yy * w * 4: off + (yy + 1) * w * 4]
        out = bytearray()
        for x in range(0, w, scale):
            B, G, R = row[x*4], row[x*4+1], row[x*4+2]
            out += bytes((R, G, B))
        rows.append(b'\x00' + bytes(out))
    W = (w + scale - 1) // scale; H = len(rows)
    def chunk(t, d): return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    png = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', W, H, 8, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(b''.join(rows), 6)) + chunk(b'IEND', b'')
    open(dst, 'wb').write(png)
    print(dst, W, H)
for s in sys.argv[1:]:
    conv(s, s[:-4] + '.png')
