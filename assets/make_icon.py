"""Draw the Notes app icon and write assets/icon.ico (+ a PNG preview).

Usage: python make_icon.py <output-dir>
"""
import sys
from pathlib import Path

from PIL import Image, ImageDraw

S = 1024  # drawn large, then downscaled for smooth edges


def gradient(top, bottom):
    img = Image.new("RGB", (S, S))
    px = img.load()
    for y in range(S):
        t = y / (S - 1)
        row = tuple(round(a + (b - a) * t) for a, b in zip(top, bottom))
        for x in range(S):
            px[x, y] = row
    return img


def draw():
    # Amber tile with rounded corners.
    tile = gradient((250, 196, 72), (240, 146, 30)).convert("RGBA")
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle((40, 40, S - 40, S - 40), radius=210, fill=255)
    icon = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    icon.paste(tile, (0, 0), mask)

    d = ImageDraw.Draw(icon)
    # Soft shadow under the sheet.
    shadow = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    ImageDraw.Draw(shadow).rounded_rectangle((262, 222, 782, 842), radius=56, fill=(120, 60, 0, 70))
    icon.alpha_composite(shadow)

    # White sheet with a folded top-right corner.
    left, top, right, bottom, fold, r = 250, 190, 774, 820, 170, 56
    sheet = Image.new("L", (S, S), 0)
    sd = ImageDraw.Draw(sheet)
    sd.rounded_rectangle((left, top, right, bottom), radius=r, fill=255)
    sd.polygon([(right - fold, top - 2), (right + 2, top - 2), (right + 2, top + fold)], fill=0)
    icon.paste(Image.new("RGBA", (S, S), (255, 253, 247, 255)), (0, 0), sheet)
    d = ImageDraw.Draw(icon)
    d.polygon(
        [(right - fold, top), (right - fold, top + fold - 40), (right - fold + 40, top + fold), (right, top + fold)],
        fill=(255, 224, 160, 255),
    )
    d.pieslice((right - fold, top + fold - 80, right - fold + 80, top + fold), 90, 180, fill=(255, 224, 160, 255))

    # Text lines: a heavier title line, then body lines.
    ink = (74, 62, 48, 255)
    faint = (74, 62, 48, 150)
    d.rounded_rectangle((330, 330, 540, 378), radius=24, fill=ink)
    for i, width in enumerate((364, 364, 250)):
        y = 470 + i * 96
        d.rounded_rectangle((330, y, 330 + width, y + 40), radius=20, fill=faint)
    return icon


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    icon = draw()
    icon.resize((256, 256), Image.LANCZOS).save(out / "icon.png")
    icon.save(out / "icon.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])


if __name__ == "__main__":
    main()
