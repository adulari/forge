#!/usr/bin/env python3
"""Render every shipped icon from the single mark in scripts/brand/forge-mark.svg.

Before this existed, each target carried its own hand-placed PNG and every one of them was the stock
Expo placeholder — the blue "A" on iOS, Android, desktop and the web manifest, and the grid-and-
circles on both splash screens. Nothing tied them together, so there was no way to change the logo
without missing one.

Run after editing the mark:

    python3 scripts/gen-brand-assets.py

Native caveat, unchanged by this script: the iOS app icon and both splash images are compiled into
the binary. Regenerating them here updates the repository, but they only reach a device through a
native build — an OTA cannot carry them.
"""

from __future__ import annotations

import re
import struct
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
MARK = REPO / "scripts/brand/forge-mark.svg"
WORK = REPO / ".brand-build"

# mobile/src/theme/tokens.ts — darkTokens.bg0 and lightTokens.bg0.
BG_DARK = "#09090B"
BG_LIGHT = "#F5F4F1"
EMBER_500 = "#F07A2E"
# The light splash needs a darker ink than the mark's own ramp: #F07A2E on #F5F4F1 measures 2.6:1,
# which is the same mistake the previous light splash made (it shipped at 1.41:1 and was invisible).
EMBER_700 = "#964916"


def inner_svg() -> str:
    """The mark's contents, minus its own <svg> wrapper, so it can be re-wrapped per target."""
    text = MARK.read_text()
    match = re.search(r"<svg[^>]*>(.*)</svg>", text, re.S)
    if match is None:
        sys.exit("could not read the mark: no <svg> element")
    return match.group(1)


def flat(body: str, colour: str) -> str:
    """Collapse the mark's gradients to one flat colour, for monochrome and single-ink variants."""
    return body.replace("url(#ember)", colour).replace("url(#hot)", colour)


def compose(body: str, *, bg: str | None, glow: bool, scale: float) -> str:
    """Wrap the mark on a canvas. `scale` is relative to the mark's native 62% of the viewport."""
    offset = 512 * (1 - scale)
    rect = f'  <rect width="1024" height="1024" fill="{bg}"/>\n' if bg else ""
    # The heat wash is a large-canvas nicety; it contributes nothing at 32px and is skipped there so
    # small icons stay crisp rather than muddy.
    heat = (
        '  <defs><radialGradient id="heat" cx="0.5" cy="0.5" r="0.5">'
        f'<stop offset="0" stop-color="{EMBER_500}" stop-opacity="0.18"/>'
        f'<stop offset="1" stop-color="{EMBER_500}" stop-opacity="0"/>'
        "</radialGradient></defs>\n"
        '  <circle cx="512" cy="512" r="470" fill="url(#heat)"/>\n'
    ) if glow else ""
    return (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">\n'
        f"{rect}{heat}"
        f'  <g transform="translate({offset:.2f} {offset:.2f}) scale({scale:.4f})">{body}</g>\n'
        "</svg>\n"
    )


def render(svg: str, out: Path, size: int) -> Path:
    out.parent.mkdir(parents=True, exist_ok=True)
    tmp = WORK / f"{out.stem}-{size}.svg"
    tmp.parent.mkdir(parents=True, exist_ok=True)
    tmp.write_text(svg)
    subprocess.run(
        ["rsvg-convert", "-w", str(size), "-h", str(size), str(tmp), "-o", str(out)],
        check=True,
    )
    return out


def force_rgba(path: Path) -> Path:
    """Keep Tauri PNGs RGBA even when their composed canvas is fully opaque."""
    tmp = WORK / f"{path.stem}-rgba.png"
    subprocess.run(
        ["magick", str(path), "-alpha", "on", f"PNG32:{tmp}"],
        check=True,
    )
    tmp.replace(path)
    return path


def write_icns(pngs: dict[str, Path], out: Path) -> None:
    """Minimal ICNS writer.

    `iconutil` is macOS-only and `png2icns` is not installed, but the container format is trivial:
    a magic, a total length, then typed entries whose payload is a PNG. Modern macOS reads PNG
    payloads for every type used here.
    """
    entries = b""
    for kind, path in pngs.items():
        data = path.read_bytes()
        entries += kind.encode("ascii") + struct.pack(">I", len(data) + 8) + data
    out.write_bytes(b"icns" + struct.pack(">I", len(entries) + 8) + entries)


def main() -> None:
    WORK.mkdir(exist_ok=True)
    body = inner_svg()

    # Full-bleed app icon: opaque, square, no rounded corners — every platform masks its own shape,
    # and pre-rounding shows as a dark halo inside the system mask on iOS.
    icon = compose(body, bg=BG_DARK, glow=True, scale=1.0)
    icon_flat = compose(body, bg=BG_DARK, glow=False, scale=1.0)

    targets: list[tuple[Path, int, str]] = [
        (REPO / "mobile/assets/icon.png", 1024, icon),
        (REPO / "mobile/ios/Forge/Images.xcassets/AppIcon.appiconset/App-Icon-1024x1024@1x.png", 1024, icon),
        (REPO / "mobile/public/icon-512.png", 512, icon),
        (REPO / "mobile/public/icon-192.png", 192, icon),
        (REPO / "docs/assets/forge-logo.png", 512, icon),
        # Favicons drop the wash: at 32px and below it is a smear, not an atmosphere.
        (REPO / "mobile/assets/favicon.png", 196, icon_flat),
        (REPO / "mobile/public/favicon.png", 196, icon_flat),
        # Tauri desktop.
        (REPO / "mobile/src-tauri/icons/icon.png", 1024, icon),
        (REPO / "mobile/src-tauri/icons/128x128@2x.png", 256, icon),
        (REPO / "mobile/src-tauri/icons/128x128.png", 128, icon),
        (REPO / "mobile/src-tauri/icons/64x64.png", 64, icon_flat),
        (REPO / "mobile/src-tauri/icons/32x32.png", 32, icon_flat),
    ]
    tauri_icons = REPO / "mobile/src-tauri/icons"
    for path, size, svg in targets:
        render(svg, path, size)
        # tauri::generate_context! rejects RGB-only icons, even when the intended canvas is opaque.
        if path.parent == tauri_icons:
            force_rgba(path)
        print(f"  {path.relative_to(REPO)} ({size}px)")

    # Android adaptive. The foreground is masked to roughly the central 66%, so the mark is pulled
    # in to 0.75 of its native size — at full size its jaw tips clip on a circular mask.
    render(compose(body, bg=None, glow=False, scale=0.75),
           REPO / "mobile/assets/android-icon-foreground.png", 1024)
    render(compose("", bg=BG_DARK, glow=False, scale=1.0),
           REPO / "mobile/assets/android-icon-background.png", 1024)
    # Themed icons recolour a silhouette, so this must be one flat ink on transparency.
    render(compose(flat(body, "#FFFFFF"), bg=None, glow=False, scale=0.75),
           REPO / "mobile/assets/android-icon-monochrome.png", 1024)
    print("  mobile/assets/android-icon-{foreground,background,monochrome}.png")

    # Splash marks: transparent, drawn at the size expo-splash-screen's imageWidth expects.
    splash_dark = compose(body, bg=None, glow=False, scale=0.92)
    splash_light = compose(flat(body, EMBER_700), bg=None, glow=False, scale=0.92)
    render(splash_dark, REPO / "mobile/assets/splash-icon.png", 512)
    render(splash_light, REPO / "mobile/assets/splash-icon-light.png", 512)

    # The same two, at the exact sizes already committed under ios/, because that directory is
    # checked in and a build will use what is there rather than re-derive it.
    imageset = REPO / "mobile/ios/Forge/Images.xcassets/SplashScreenLogo.imageset"
    for name, svg in (("image", splash_light), ("dark_image", splash_dark)):
        for suffix, size in (("", 200), ("@2x", 400), ("@3x", 600)):
            render(svg, imageset / f"{name}{suffix}.png", size)
    print("  mobile/assets/splash-icon{,-light}.png + ios SplashScreenLogo.imageset")

    # .ico for the web favicon and Tauri's Windows icon.
    ico_sizes = [16, 32, 48, 64, 128, 256]
    ico_pngs = [render(icon_flat if s <= 64 else icon, WORK / f"ico-{s}.png", s) for s in ico_sizes]
    for out in (REPO / "mobile/src-tauri/icons/icon.ico", REPO / "mobile/public/favicon.ico"):
        subprocess.run(["magick", *[str(p) for p in ico_pngs], str(out)], check=True)
        print(f"  {out.relative_to(REPO)}")

    # .icns for macOS.
    icns_types = {"ic07": 128, "ic08": 256, "ic09": 512, "ic10": 1024,
                  "ic11": 32, "ic12": 64, "ic13": 256, "ic14": 512}
    pngs = {k: render(icon_flat if v <= 64 else icon, WORK / f"icns-{k}.png", v)
            for k, v in icns_types.items()}
    write_icns(pngs, REPO / "mobile/src-tauri/icons/icon.icns")
    print("  mobile/src-tauri/icons/icon.icns")


if __name__ == "__main__":
    main()
