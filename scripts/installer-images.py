"""Generates the installer artwork (NSIS + WiX) from the app icon. Run: python scripts/installer-images.py"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "src-tauri" / "installer"
ICON = Image.open(ROOT / "src-tauri" / "icons" / "icon.png").convert("RGBA")
FONTS = Path("C:/Windows/Fonts")

DARK = (22, 22, 22)
FG = (235, 235, 235)
MUTED = (150, 150, 150)
WHITE = (255, 255, 255)


def font(size: int, bold: bool = False) -> ImageFont.FreeTypeFont:
    return ImageFont.truetype(str(FONTS / ("segoeuib.ttf" if bold else "segoeui.ttf")), size)


def paste_icon(img: Image.Image, size: int, xy: tuple[int, int]) -> None:
    icon = ICON.resize((size, size), Image.LANCZOS)
    img.paste(icon, xy, icon)


def center_text(draw: ImageDraw.ImageDraw, width: int, y: int, text: str, f, fill) -> None:
    w = draw.textlength(text, font=f)
    draw.text(((width - w) / 2, y), text, font=f, fill=fill)


def save(img: Image.Image, name: str) -> None:
    # NSIS and WiX want plain 24-bit BMPs.
    img.convert("RGB").save(OUT / name, "BMP")


def brand_panel(img: Image.Image, width: int) -> None:
    """Dark left panel: logo, name and tagline."""
    d = ImageDraw.Draw(img)
    d.rectangle((0, 0, width - 1, img.height - 1), fill=DARK)
    paste_icon(img, 84, ((width - 84) // 2, 78))
    center_text(d, width, 176, "AgentDeck", font(20, bold=True), FG)
    center_text(d, width, 204, "Your coding agents,", font(11), MUTED)
    center_text(d, width, 219, "side by side", font(11), MUTED)


def nsis_sidebar() -> None:
    """Welcome/finish pages, left panel (the text is drawn on white to the right of it)."""
    img = Image.new("RGB", (164, 314), DARK)
    brand_panel(img, 164)
    save(img, "nsis-sidebar.bmp")


def nsis_header() -> None:
    """Top banner of the inner pages (shown at the right of a white header)."""
    w = 150
    img = Image.new("RGB", (w, 57), WHITE)
    d = ImageDraw.Draw(img)
    paste_icon(img, 40, (w - 48, 8))
    f = font(15, bold=True)
    d.text((w - 54 - d.textlength("AgentDeck", font=f), 17), "AgentDeck", font=f, fill=DARK)
    save(img, "nsis-header.bmp")


def wix_banner() -> None:
    """Top banner of the MSI dialogs: Windows draws black text on its left part, so keep it white."""
    w = 493
    img = Image.new("RGB", (w, 58), WHITE)
    paste_icon(img, 42, (w - 54, 8))
    save(img, "wix-banner.bmp")


def wix_dialog() -> None:
    """Welcome/finish background of the MSI: dark panel on the left, white where the text goes."""
    img = Image.new("RGB", (493, 312), WHITE)
    brand_panel(img, 164)
    save(img, "wix-dialog.bmp")


if __name__ == "__main__":
    OUT.mkdir(exist_ok=True)
    for build in (nsis_sidebar, nsis_header, wix_banner, wix_dialog):
        build()
    for f in sorted(OUT.glob("*.bmp")):
        print(f.name, Image.open(f).size)
