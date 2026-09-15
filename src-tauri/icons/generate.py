"""Draws Toglet's icon artwork and the raw tray images.

Run from the repository root, then let the Tauri CLI derive the platform icon sets:

    python3 src-tauri/icons/generate.py
    pnpm exec tauri icon src-tauri/icons/app-icon.png --output src-tauri/icons

Needs Pillow. Nothing at runtime depends on this script; the generated files are checked in.

The mark is the quota ring: a dark tile with a green arc open at the upper left. The app icon
follows the macOS layout - the tile covers 824 of a 1024 canvas with a 22.5% corner radius - so
it sits at the same size and shape as its neighbours in the Dock. The same file drives the
Windows icon, where the margin is simply part of the image.

The tray images are written as raw RGBA rather than PNG because the app decodes no image
formats; adding a decoder for two tiny icons is not worth a dependency. The menu bar one is a
template (black plus alpha) so macOS can tint it for light and dark menu bars; the Windows one
keeps the tile, since the tray there does not recolour icons and a bare ring would vanish on a
light taskbar.
"""

import math
from pathlib import Path

from PIL import Image, ImageDraw

HERE = Path(__file__).parent

TILE = (11, 12, 13, 255)
GREEN = (64, 230, 164, 255)
BLACK = (0, 0, 0, 255)

# Ring proportions relative to the tile's half-width, measured from the original artwork.
OUTER = 0.676
STROKE = 0.137
# Where the arc ends, in degrees clockwise from three o'clock (Pillow's convention). The gap
# between the two is at the upper left.
ARC_START = 257
ARC_END = 173

# Drawn at this multiple of the target size and downsampled: Pillow's arc has no anti-aliasing.
SUPERSAMPLE = 8


def ring(size: int, half: float, centre: float, colour: tuple[int, int, int, int]) -> Image.Image:
    """The arc with round caps on a transparent canvas of `size` pixels."""
    s = SUPERSAMPLE
    layer = Image.new("RGBA", (size * s, size * s), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)
    outer = OUTER * half * s
    stroke = STROKE * half * s
    mid = outer - stroke / 2
    c = centre * s
    box = (c - outer, c - outer, c + outer, c + outer)
    draw.arc(box, ARC_START, ARC_END, fill=colour, width=round(stroke))
    for angle in (ARC_START, ARC_END):
        a = math.radians(angle)
        x, y = c + mid * math.cos(a), c + mid * math.sin(a)
        r = stroke / 2
        draw.ellipse((x - r, y - r, x + r, y + r), fill=colour)
    return layer.resize((size, size), Image.LANCZOS)


def tile(size: int, inset: int, radius: float) -> Image.Image:
    """A rounded dark square inset from the canvas edge."""
    s = SUPERSAMPLE
    layer = Image.new("RGBA", (size * s, size * s), (0, 0, 0, 0))
    ImageDraw.Draw(layer).rounded_rectangle(
        (inset * s, inset * s, (size - inset) * s - 1, (size - inset) * s - 1),
        radius=radius * s,
        fill=TILE,
    )
    return layer.resize((size, size), Image.LANCZOS)


def app_icon() -> None:
    size, inset = 1024, 100
    side = size - 2 * inset
    image = tile(size, inset, side * 0.225)
    image.alpha_composite(ring(size, side / 2, size / 2, GREEN))
    image.save(HERE / "app-icon.png")


def raw(image: Image.Image, name: str) -> None:
    (HERE / name).write_bytes(image.tobytes("raw", "RGBA"))


def tray_icons() -> None:
    # Menu bar: 18pt tall on macOS, supplied at 2x. Black on transparent; the system tints it.
    size = 36
    raw(ring(size, size / 2, size / 2, BLACK), "tray-macos.rgba")
    # Taskbar: the tile without the Dock margin, so the mark fills the 16px slot it gets.
    size = 32
    image = tile(size, 0, size * 0.225)
    image.alpha_composite(ring(size, size / 2, size / 2, GREEN))
    raw(image, "tray-windows.rgba")


if __name__ == "__main__":
    app_icon()
    tray_icons()
