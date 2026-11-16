#!/usr/bin/env python3
"""Generate the Vibe Terminal brand assets. Requires Pillow only when regenerating.

Normal Cargo builds use the checked-in images and do not run this script.
The vector and bitmap versions share the same geometry and colors below.
"""
from pathlib import Path
from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "assets"
ICONS = ASSETS / "icons"
MINT = "#b4edc5"
INK = "#181818"
ICON_SIZES = (16, 20, 24, 32, 40, 48, 64, 128, 256)
PROMPT = ((14, 19), (28, 32), (14, 45))
CURSOR = ((35, 45), (50, 45))


def render(size: int) -> Image.Image:
    # Supersampling preserves the rounded joins and transparent outside corners
    # at Windows taskbar, high-DPI and Explorer thumbnail sizes.
    scale = max(4, 1024 // size)
    canvas_size = size * scale
    unit = canvas_size / 64
    image = Image.new("RGBA", (canvas_size, canvas_size), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle((0, 0, canvas_size - 1, canvas_size - 1), radius=16 * unit, fill=MINT)
    width = round(5 * unit)
    radius = width / 2
    for points in (PROMPT, CURSOR):
        scaled = [(x * unit, y * unit) for x, y in points]
        draw.line(scaled, fill=INK, width=width, joint="curve")
        for x, y in scaled:
            draw.ellipse((x - radius, y - radius, x + radius, y + radius), fill=INK)
    return image.resize((size, size), Image.Resampling.LANCZOS)


def main() -> None:
    ICONS.mkdir(parents=True, exist_ok=True)
    svg = (
        '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64">'
        f'<rect width="64" height="64" rx="16" fill="{MINT}"/>'
        f'<path d="m14 19 14 13-14 13m21 0h15" fill="none" stroke="{INK}" '
        'stroke-width="5" stroke-linecap="round" stroke-linejoin="round"/></svg>\n'
    )
    (ASSETS / "logo.svg").write_text(svg, encoding="utf-8")
    render(128).save(ASSETS / "logo.png")
    render(1024).save(ICONS / "app-icon.png")
    frames = [render(size) for size in ICON_SIZES]
    frames[-1].save(ICONS / "app.ico", format="ICO", sizes=[(n, n) for n in ICON_SIZES], append_images=frames[:-1])
    # Validate every embedded frame rather than merely checking the extension.
    with Image.open(ICONS / "app.ico") as icon:
        assert icon.ico.sizes() == {(size, size) for size in ICON_SIZES}
        for size in ICON_SIZES:
            frame = icon.ico.getimage((size, size))
            assert frame.size == (size, size)
            # Lanczos filtering can leave a sub-2% alpha fringe at tiny sizes.
            assert frame.getpixel((0, 0))[3] <= 4
    print("Generated logo.svg, logo.png, icons/app-icon.png and icons/app.ico")
    print("ICO sizes: " + ", ".join(str(size) for size in ICON_SIZES))


if __name__ == "__main__":
    main()
