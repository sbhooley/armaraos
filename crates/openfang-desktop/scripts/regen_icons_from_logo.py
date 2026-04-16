#!/usr/bin/env python3
"""Regenerate desktop bundle icons and embedded web assets from public/assets/armaraos-logo.png.

Letterboxes the source PNG onto a transparent square, then resizes for each bundle asset.
Run from repo root: python3 crates/openfang-desktop/scripts/regen_icons_from_logo.py
"""
from __future__ import annotations

import subprocess
import sys
import tempfile
from pathlib import Path

from PIL import Image

REPO = Path(__file__).resolve().parents[3]
ICONS = REPO / "crates" / "openfang-desktop" / "icons"
LOGO = REPO / "public" / "assets" / "armaraos-logo.png"


def fit_transparent_square(im: Image.Image, side: int = 1024) -> Image.Image:
    """Scale `im` uniformly to fit inside `side`×`side`, centered on a transparent square."""
    im = im.convert("RGBA")
    w, h = im.size
    scale = min(side / w, side / h)
    nw = max(1, int(round(w * scale)))
    nh = max(1, int(round(h * scale)))
    resized = im.resize((nw, nh), Image.Resampling.LANCZOS)
    out = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    x = (side - nw) // 2
    y = (side - nh) // 2
    out.paste(resized, (x, y), resized)
    return out


def main() -> int:
    if not LOGO.is_file():
        print(f"Missing logo: {LOGO}", file=sys.stderr)
        return 1

    with Image.open(LOGO) as raw:
        src = fit_transparent_square(raw, 1024)

    try:
        for path in sorted(ICONS.rglob("*.png")):
            with Image.open(path) as target:
                w, h = target.size
            out = src.resize((w, h), Image.Resampling.LANCZOS)
            out.save(path, "PNG")
            print("updated", path.relative_to(REPO))

        iconset = Path(tempfile.mkdtemp(suffix=".iconset"))
        try:
            pairs = [
                (16, "icon_16x16.png"),
                (32, "icon_16x16@2x.png"),
                (32, "icon_32x32.png"),
                (64, "icon_32x32@2x.png"),
                (128, "icon_128x128.png"),
                (256, "icon_128x128@2x.png"),
                (256, "icon_256x256.png"),
                (512, "icon_256x256@2x.png"),
                (512, "icon_512x512.png"),
                (1024, "icon_512x512@2x.png"),
            ]
            for dim, name in pairs:
                src.resize((dim, dim), Image.Resampling.LANCZOS).save(iconset / name, "PNG")
            icns_out = ICONS / "icon.icns"
            subprocess.run(
                ["iconutil", "-c", "icns", str(iconset), "-o", str(icns_out)],
                check=True,
            )
            print("updated", icns_out.relative_to(REPO))
        finally:
            for p in iconset.glob("*.png"):
                p.unlink(missing_ok=True)
            iconset.rmdir()

        ico_sizes = (16, 24, 32, 48, 64, 128, 256)
        ico_imgs = [src.resize((s, s), Image.Resampling.LANCZOS) for s in ico_sizes]
        ico_path = ICONS / "icon.ico"
        ico_imgs[0].save(
            ico_path,
            format="ICO",
            sizes=[(s, s) for s in ico_sizes],
            append_images=ico_imgs[1:],
        )
        print("updated", ico_path.relative_to(REPO))

        static_dir = REPO / "crates" / "openfang-api" / "static"
        assets_dir = static_dir / "assets"
        assets_dir.mkdir(parents=True, exist_ok=True)

        src.resize((512, 512), Image.Resampling.LANCZOS).save(static_dir / "logo.png", "PNG")
        print("updated", (static_dir / "logo.png").relative_to(REPO))

        src.resize((1024, 1024), Image.Resampling.LANCZOS).save(
            assets_dir / "armaraos-logo.png",
            "PNG",
        )
        print("updated", (assets_dir / "armaraos-logo.png").relative_to(REPO))

        fav_sizes = (16, 24, 32, 48, 64)
        fav_imgs = [src.resize((s, s), Image.Resampling.LANCZOS) for s in fav_sizes]
        fav_path = static_dir / "favicon.ico"
        fav_imgs[0].save(
            fav_path,
            format="ICO",
            sizes=[(s, s) for s in fav_sizes],
            append_images=fav_imgs[1:],
        )
        print("updated", fav_path.relative_to(REPO))
    finally:
        src.close()

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
