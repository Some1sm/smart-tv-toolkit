"""Draw the Smart TV Toolkit icon and write every size the builds need.

    uv run --with pillow python scripts/make_icons.py

Writes assets/icon.png (1024 px), assets/icon-256.png (window icon),
assets/icon.ico (Windows) and assets/icon.icns (macOS).
"""
from pathlib import Path

from PIL import Image, ImageDraw

# One Dark palette, matching src/theme.rs
BG = (33, 37, 43)          # SIDEBAR_BG
SCREEN = (44, 49, 60)      # SURFACE_1
FRAME = (97, 175, 239)     # PRIMARY
TRACK = (75, 82, 99)       # BORDER
KNOBS = [(152, 195, 121), (229, 192, 123), (224, 108, 117)]  # SUCCESS, WARNING, DANGER

SIZE = 1024
SCALE = 4  # draw large, then downsample for smooth edges


def draw() -> Image.Image:
    s = SIZE * SCALE
    img = Image.new("RGBA", (s, s), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    u = s / 1024  # design units: the icon is laid out on a 1024 grid

    def box(x0, y0, x1, y1):
        return [x0 * u, y0 * u, x1 * u, y1 * u]

    # Rounded-square tile (macOS-style margins so it doesn't look oversized).
    d.rounded_rectangle(box(80, 80, 944, 944), radius=190 * u, fill=BG)

    # TV body and screen.
    d.rounded_rectangle(box(190, 250, 834, 690), radius=56 * u, fill=FRAME)
    d.rounded_rectangle(box(222, 282, 802, 658), radius=34 * u, fill=SCREEN)

    # Stand.
    d.polygon([(452 * u, 690 * u), (572 * u, 690 * u), (612 * u, 770 * u), (412 * u, 770 * u)], fill=FRAME)
    d.rounded_rectangle(box(352, 762, 672, 800), radius=19 * u, fill=FRAME)

    # Settings sliders on the screen: the "toolkit".
    for i, (knob_x, color) in enumerate(zip((620, 400, 540), KNOBS)):
        y = 370 + i * 100
        d.rounded_rectangle(box(300, y - 9, 724, y + 9), radius=9 * u, fill=TRACK)
        d.rounded_rectangle(box(300, y - 9, knob_x, y + 9), radius=9 * u, fill=color)
        d.ellipse(box(knob_x - 34, y - 34, knob_x + 34, y + 34), fill=color)
        d.ellipse(box(knob_x - 14, y - 14, knob_x + 14, y + 14), fill=SCREEN)

    return img.resize((SIZE, SIZE), Image.LANCZOS)


def main() -> None:
    assets = Path(__file__).resolve().parent.parent / "assets"
    assets.mkdir(exist_ok=True)
    icon = draw()
    icon.save(assets / "icon.png")
    icon.resize((256, 256), Image.LANCZOS).save(assets / "icon-256.png")
    icon.save(assets / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    icon.save(assets / "icon.icns")
    for f in sorted(assets.iterdir()):
        print(f"{f.name:14} {f.stat().st_size:>8} bytes")


if __name__ == "__main__":
    main()
