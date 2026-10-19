#!/usr/bin/env python3
"""Package the canonical GateTree icon with its upper-right X into desktop formats.

The checked-in source is independent of the original GateTree application.
Regenerating sizes preserves the approved artwork and its transparent corners.
"""
from pathlib import Path
from PIL import Image

ROOT = Path(__file__).resolve().parents[1]
ICONS = ROOT / "src-tauri" / "icons"
SOURCE = ICONS / "gatetree-ops-source.png"


def main() -> None:
    with Image.open(SOURCE) as source:
        if source.width != source.height or source.width < 1024:
            raise ValueError("The canonical icon must be square and at least 1024 pixels.")
        icon = source.convert("RGBA").resize((1024, 1024), Image.Resampling.LANCZOS)
    icon.save(ICONS / "icon.png")
    for size, filename in ((32, "32x32.png"), (128, "128x128.png"), (256, "128x128@2x.png")):
        icon.resize((size, size), Image.Resampling.LANCZOS).save(ICONS / filename)
    icon.save(ICONS / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    icon.save(ICONS / "icon.icns")


if __name__ == "__main__":
    main()
