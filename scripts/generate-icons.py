#!/usr/bin/env python3
"""Generate the PHAROS desktop icon set from the canonical beacon geometry."""

from pathlib import Path
from PIL import Image, ImageDraw, ImageFilter


ROOT = Path(__file__).resolve().parents[1]
ICONS = ROOT / "src-tauri" / "icons"
SCALE = 4
SIZE = 1024


def scaled(value: int) -> int:
    return value * SCALE


def draw_icon() -> Image.Image:
    canvas = Image.new("RGBA", (scaled(SIZE), scaled(SIZE)), (0, 0, 0, 0))
    tile = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    tile_draw = ImageDraw.Draw(tile)
    tile_draw.rounded_rectangle(
        (scaled(58), scaled(58), scaled(966), scaled(966)),
        radius=scaled(205),
        fill=(7, 21, 34, 255),
    )
    canvas.alpha_composite(tile)

    glow = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
    glow_draw = ImageDraw.Draw(glow)
    cyan = (89, 226, 232, 215)
    glow_draw.line(
        [(scaled(512), scaled(682)), (scaled(512), scaled(356))],
        fill=cyan,
        width=scaled(52),
    )
    glow_draw.arc(
        (scaled(512), scaled(293), scaled(638), scaled(419)),
        180,
        360,
        fill=cyan,
        width=scaled(52),
    )
    glow_draw.line(
        [(scaled(638), scaled(356)), (scaled(638), scaled(682))],
        fill=cyan,
        width=scaled(52),
    )
    glow_draw.line(
        [(scaled(448), scaled(682)), (scaled(702), scaled(682))],
        fill=cyan,
        width=scaled(52),
    )
    canvas.alpha_composite(glow.filter(ImageFilter.GaussianBlur(scaled(30))))

    draw = ImageDraw.Draw(canvas)
    stroke = (89, 226, 232, 255)
    width = scaled(30)
    draw.line([(scaled(512), scaled(682)), (scaled(512), scaled(356))], fill=stroke, width=width)
    draw.arc((scaled(512), scaled(293), scaled(638), scaled(419)), 180, 360, fill=stroke, width=width)
    draw.line([(scaled(638), scaled(356)), (scaled(638), scaled(682))], fill=stroke, width=width)
    draw.line([(scaled(448), scaled(682)), (scaled(702), scaled(682))], fill=stroke, width=width)

    ray = (89, 226, 232, 210)
    ray_width = scaled(18)
    draw.line([(scaled(235), scaled(345)), (scaled(432), scaled(415))], fill=ray, width=ray_width)
    draw.line([(scaled(204), scaled(510)), (scaled(432), scaled(510))], fill=ray, width=ray_width)
    draw.line([(scaled(235), scaled(675)), (scaled(432), scaled(605))], fill=ray, width=ray_width)

    return canvas.resize((SIZE, SIZE), Image.Resampling.LANCZOS)


def main() -> None:
    ICONS.mkdir(parents=True, exist_ok=True)
    icon = draw_icon()
    icon.save(ICONS / "icon.png")
    icon.resize((32, 32), Image.Resampling.LANCZOS).save(ICONS / "32x32.png")
    icon.resize((128, 128), Image.Resampling.LANCZOS).save(ICONS / "128x128.png")
    icon.resize((256, 256), Image.Resampling.LANCZOS).save(ICONS / "128x128@2x.png")
    icon.save(ICONS / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
    icon.save(ICONS / "icon.icns")


if __name__ == "__main__":
    main()
