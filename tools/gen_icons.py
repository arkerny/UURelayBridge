# -*- coding: utf-8 -*-
"""Generate all RelayBridge icon assets (app icon, .ico, .icns, tray icons)."""
import os, math
from PIL import Image, ImageDraw

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ICONS = os.path.join(ROOT, "icons")
for sub in ("windows", "macos", "shared", "tray"):
    os.makedirs(os.path.join(ICONS, sub), exist_ok=True)

BG = (26, 32, 45, 255)
FG = (255, 255, 255, 255)
ACCENT = (47, 214, 195, 255)
OFFC = (138, 147, 166, 255)


def bridge(d, w, h, color, stroke, pad=0.0, baseline=True):
    """A minimal bridge glyph inside a w x h box."""
    x0, y0 = w * (0.16 + pad), h * (0.30 + pad)
    x1, y1 = w * (0.84 - pad), h * (0.78 - pad)
    mid = (x0 + x1) / 2
    # deck
    d.line([(x0, y1), (x1, y1)], fill=color, width=stroke)
    # arch
    box = [x0, y0 - (y1 - y0) * 0.55, x1, y1 + (y1 - y0) * 0.55]
    d.arc(box, start=200, end=340, fill=color, width=stroke)
    # piers
    for px in (x0 + (x1 - x0) * 0.16, mid, x1 - (x1 - x0) * 0.16):
        d.line([(px, y1), (px, y1 + (y1 - y0) * 0.16)], fill=color, width=stroke)


def app_icon(size=1024):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    r = int(size * 0.22)
    d.rounded_rectangle([0, 0, size - 1, size - 1], radius=r, fill=BG)
    # subtle accent ring
    d.rounded_rectangle([int(size*0.055)]*2 + [size - 1 - int(size*0.055)]*2,
                        radius=int(size * 0.18), outline=(255, 255, 255, 26),
                        width=max(2, size // 256))
    bridge(d, size, size, ACCENT, stroke=max(6, int(size * 0.028)))
    return img


def tray_icon(size=64, on=True):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    col = ACCENT if on else OFFC
    # filled disc + white glyph
    d.ellipse([2, 2, size - 3, size - 3], fill=col)
    if on:
        bridge(d, size, size, (17, 24, 32, 255), stroke=max(2, size // 16), pad=0.04)
    else:
        # slash = disabled
        bridge(d, size, size, (240, 243, 248, 255), stroke=max(2, size // 16), pad=0.04)
        d.line([(size * 0.16, size * 0.16), (size * 0.84, size * 0.84)],
               fill=(240, 243, 248, 255), width=max(3, size // 12))
    return img


def template_icon(size=44, on=True):
    """macOS menu bar template: pure black + alpha, no background."""
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    col = (0, 0, 0, 255)
    bridge(d, size, size, col, stroke=max(2, size // 14), pad=0.06)
    if not on:
        d.line([(size * 0.12, size * 0.12), (size * 0.88, size * 0.88)],
               fill=col, width=max(2, size // 16))
    return img


def save(img, name):
    p = os.path.join(ICONS, name)
    img.save(p)
    print("wrote", p, img.size)


# ---- app icons
app = app_icon(1024)
save(app, "icon.png")
app.resize((256, 256), Image.LANCZOS).save(os.path.join(ICONS, "128x128@2x.png"))
app.resize((128, 128), Image.LANCZOS).save(os.path.join(ICONS, "128x128.png"))
app.resize((32, 32), Image.LANCZOS).save(os.path.join(ICONS, "32x32.png"))
app.resize((64, 64), Image.LANCZOS).save(os.path.join(ICONS, "64x64.png"))

# ---- windows .ico (multi-size, PNG-compressed entries)
sizes = [16, 24, 32, 48, 64, 128, 256]
frames = [app.resize((s, s), Image.LANCZOS) for s in sizes]
ico_path = os.path.join(ICONS, "icon.ico")
frames[-1].save(ico_path, format="ICO", sizes=[(s, s) for s in sizes])
print("wrote", ico_path)

# ---- macOS .icns (PNG-based chunks)
icns_entries = [(b"ic07", 128), (b"ic08", 256), (b"ic09", 512),
                (b"ic10", 1024), (b"ic11", 32), (b"ic12", 64),
                (b"ic13", 256), (b"ic14", 512)]
chunks = []
for typ, s in icns_entries:
    buf = os.path.join(ICONS, "_tmp.png")
    app.resize((s, s), Image.LANCZOS).save(buf, format="PNG")
    data = open(buf, "rb").read()
    os.remove(buf)
    chunks.append(typ + (len(data) + 8).to_bytes(4, "big") + data)
body = b"".join(chunks)
icns_path = os.path.join(ICONS, "icon.icns")
with open(icns_path, "wb") as f:
    f.write(b"icns" + (len(body) + 8).to_bytes(4, "big") + body)
print("wrote", icns_path, os.path.getsize(icns_path), "bytes")

# ---- tray icons
save(tray_icon(64, True), "tray/tray-on.png")
save(tray_icon(64, False), "tray/tray-off.png")
save(tray_icon(32, True), "tray/tray-on@32.png")
save(tray_icon(32, False), "tray/tray-off@32.png")
save(template_icon(44, True), "tray/tray-on-template.png")
save(template_icon(44, False), "tray/tray-off-template.png")
save(template_icon(22, True), "tray/tray-on-template@1x.png")
save(template_icon(22, False), "tray/tray-off-template@1x.png")
print("done")
