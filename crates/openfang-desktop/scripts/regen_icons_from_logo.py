#!/usr/bin/env python3
"""Regenerate crates/openfang-desktop/icons/* from public/assets/armaraos-logo.png.

Pads the marketing PNG to a square (black) then resizes for each bundle asset.
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
SKIP_NAMES = frozenset({"armaraos-logo.png"})


def main() -> int:
    if not LOGO.is_file():
        print(f"Missing logo: {LOGO}", file=sys.stderr)
        return 1

    with tempfile.NamedTemporaryFile(suffix=".png", delete=False) as tmp:
        square_path = Path(tmp.name)

    subprocess.run(
        [
            "sips",
            "-p",
            "1024",
            "1024",
            "--padColor",
            "000000",
            str(LOGO),
            "--out",
            str(square_path),
        ],
        check=True,
    )

    src = Image.open(square_path).convert("RGBA")
    try:
        for path in sorted(ICONS.rglob("*.png")):
            if path.name in SKIP_NAMES:
                continue
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
    finally:
        src.close()
        square_path.unlink(missing_ok=True)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
