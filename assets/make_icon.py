"""Draws the Nimble icon: a charcoal tile with a white N whose diagonal is
an orange slash. Every size is drawn on its own pixel grid (no downscaling
of one big image), so stems land on whole pixels and the N is exactly
centered at 16 px as well as 256.

Writes ../nimble.ico (embedded in the exe) and nimble.png (README).
Run with: python assets/make_icon.py
"""
from PIL import Image, ImageDraw
import os

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)

TILE = (38, 38, 38)
EDGE = (68, 68, 68)      # keeps the tile visible on dark taskbars
WHITE = (255, 255, 255)
ORANGE = (232, 85, 30)

ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
SS = 16  # supersampling factor


def same_parity(v, s):
    """Nearest integer to v with the parity of s, so (s - result) / 2 is whole."""
    n = round(v)
    if (n - s) % 2:
        n += 1 if v > n else -1
    return n


def draw(s):
    """The icon at s x s pixels."""
    k = SS
    im = Image.new("RGBA", (s * k, s * k), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)

    # Tile: fills the square at small sizes, a little margin when large.
    inset = 0 if s <= 24 else round(s * 0.03)
    radius = s * 0.22
    edge = max(1.0, s / 128)  # hairline, at least one pixel
    box = [inset * k, inset * k, (s - inset) * k - 1, (s - inset) * k - 1]
    d.rounded_rectangle(box, radius=radius * k, fill=EDGE)
    e = edge * k
    d.rounded_rectangle([box[0] + e, box[1] + e, box[2] - e, box[3] - e], radius=(radius - edge) * k, fill=TILE)

    # N: about half the tile, whole-pixel stems, exactly centered.
    w = same_parity(s * 0.50, s)
    h = same_parity(s * 0.52, s)
    stem = max(2, round(s * 0.135))
    x0 = (s - w) // 2
    y0 = (s - h) // 2
    x1, y1 = x0 + w, y0 + h
    px = lambda *v: [c * k for c in v]
    d.rectangle(px(x0, y0, x0 + stem, y1), fill=WHITE)
    d.rectangle(px(x1 - stem, y0, x1, y1), fill=WHITE)
    d.polygon([tuple(px(x0, y0)), tuple(px(x0 + stem, y0)), tuple(px(x1, y1)), tuple(px(x1 - stem, y1))], fill=ORANGE)

    return im.resize((s, s), Image.BOX)


def main():
    images = [draw(s) for s in ICO_SIZES]
    largest = images[-1]
    largest.save(
        os.path.join(ROOT, "nimble.ico"),
        sizes=[(s, s) for s in ICO_SIZES],
        append_images=images[:-1],
    )
    draw(1024).save(os.path.join(HERE, "nimble.png"))


if __name__ == "__main__":
    main()
