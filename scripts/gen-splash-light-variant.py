#!/usr/bin/env python3
"""Re-ink the dark-theme splash mark for the light-theme splash background.

`mobile/assets/splash-icon.png` is drawn in a light gray for the dark splash (#09090B). Composited
on the light splash (#F5F4F1) it measures 1.41:1 — effectively invisible. This produces the light
variant: same geometry, inked in `lightTokens.ink`, with each pixel's original brightness carried
into alpha so the mark's internal weighting survives the inversion (on a dark bg a brighter pixel
reads as stronger ink; on a light bg that role belongs to opacity).

Run after changing the source mark. Both assets are baked in at prebuild, so a native build is
required to ship either — an OTA cannot update them.
"""

from pathlib import Path

from PIL import Image

REPO = Path(__file__).resolve().parent.parent
SOURCE = REPO / "mobile/assets/splash-icon.png"
TARGET = REPO / "mobile/assets/splash-icon-light.png"

INK = (0x1C, 0x1B, 0x19)  # mobile/src/theme/tokens.ts lightTokens.ink
BRIGHTEST = 221  # the source mark's brightest tier, which becomes fully-weighted ink


def main() -> None:
    src = Image.open(SOURCE).convert("RGBA")
    width, height = src.size
    read = src.load()
    out = Image.new("RGBA", (width, height))
    write = out.load()

    for y in range(height):
        for x in range(width):
            r, g, b, a = read[x, y]
            if a == 0:
                write[x, y] = (0, 0, 0, 0)
                continue
            brightness = (r + g + b) / 3
            write[x, y] = (*INK, min(255, round(a * min(1.0, brightness / BRIGHTEST))))

    out.save(TARGET, optimize=True)
    print(f"wrote {TARGET.relative_to(REPO)}")


if __name__ == "__main__":
    main()
