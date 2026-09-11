"""
Generate the full Bilusic icon set from the AI-generated master.

Reads the 1024x1024 PNG (with model watermark in the gray border around the
squircle icon), auto-crops the icon shape itself, then writes every size and
format Tauri 2 expects to apps/desktop/src-tauri/icons/.
"""
from __future__ import annotations

from pathlib import Path
from PIL import Image, ImageDraw, ImageOps

SOURCE_DIR = Path("/Users/guho/Desktop/Developer/bilusic/apps/desktop/src-tauri/icons/_source")
OUT_DIR = Path("/Users/guho/Desktop/Developer/bilusic/apps/desktop/src-tauri/icons")
# v3 master (2026-08-19): Bilusic wordmark over a faded TV-robot watermark
# in coral pink #EC4899. Wordmark dominates the front; TV silhouette sits
# behind as a soft pink ghost. Replaces v2 "editorial play × wave".
MASTER_PATH = SOURCE_DIR / "bilusic_wordmark_2026-08-19.png"


def auto_crop_icon(png_path: Path) -> Image.Image:
    """Crop the generated PNG to its rounded-square icon (drops watermark margin).

    The model renders the icon at roughly 76% of the canvas size, centered,
    with a light gray border around it (and the AI watermark in the bottom-
    right corner of that border). Hard-cropping at 12% inset on every edge
    is the simplest robust crop — color-based bbox thresholds fail because
    the watermark text is dark on light gray and gets picked up.
    """
    # IMPORTANT: keep RGBA. Tauri 2's generate_context!() reads every PNG in
    # `bundle.icon` at compile time and rejects non-RGBA files. convert("RGB")
    # silently drops alpha and bricks the dev build.
    img = Image.open(png_path).convert("RGBA")
    side = int(min(img.size) * 0.76)
    cx, cy = img.size[0] // 2, img.size[1] // 2
    half = side // 2
    return img.crop((cx - half, cy - half, cx + half, cy + half))


def _ensure_rgba(img: Image.Image) -> Image.Image:
    """Defensive: flatten to RGBA so every downstream write produces RGBA PNGs.

    PIL's resize preserves mode, so as long as the source is RGBA everything
    downstream is fine — but `.convert("RGBA")` on an already-RGBA image is a
    cheap no-op for our purposes and is the safest single guardrail.
    """
    if img.mode != "RGBA":
        return img.convert("RGBA")
    return img


def apply_squircle_mask(icon: Image.Image, corner_ratio: float = 0.22337) -> Image.Image:
    """Round the corners of the icon's content bbox to a macOS Big Sur squircle.

    Why this exists: macOS only auto-applies its rounded-corner mask to PNGs
    whose non-transparent content fills the canvas. Our `master_with_padding`
    centers a ~76% subject with ~12% transparent border so the dock's own
    visual padding stacks on top without doubling up — but that means macOS
    sees the PNG as "not full-bleed" and skips the system rounding, leaving
    the icon as a hard square. The AI master itself ships hand-drawn squircle
    edges (Apple-style rounded square), but they get rendered straight at the
    dock tile size, so visually you still get a rectangle.

    Fix: explicitly round the subject's corners with a Big Sur squircle mask
    (~22.37% corner ratio) so the dock shows a rounded icon at the same
    visual size the original script's 76% padding was tuned for.
    """
    icon = _ensure_rgba(icon)
    alpha = icon.split()[-1]
    bbox = alpha.getbbox()
    if not bbox:
        return icon
    w, h = bbox[2] - bbox[0], bbox[3] - bbox[1]
    side = max(w, h)
    if side == 0:
        return icon
    radius = int(side * corner_ratio)

    # Mask drawn on a square side×side canvas, then composited onto the
    # cropped subject so the corners outside the rounded rect become
    # transparent. The cropped region is square-cropped (no aspect distortion)
    # before the mask is applied.
    mask = Image.new("L", (side, side), 0)
    ImageDraw.Draw(mask).rounded_rectangle(
        (0, 0, side - 1, side - 1), radius=radius, fill=255
    )

    # Square-crop to bbox (longest side wins) so the mask fits perfectly.
    cx, cy = (bbox[0] + bbox[2]) // 2, (bbox[1] + bbox[3]) // 2
    half = side // 2
    cropped = icon.crop((cx - half, cy - half, cx + half, cy + half))
    if cropped.size != (side, side):
        cropped = cropped.resize((side, side), Image.LANCZOS)

    rounded = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    rounded.paste(cropped, (0, 0), mask)
    return rounded


def square_to_size(img: Image.Image, size: int) -> Image.Image:
    """Resize a square image to `size`x`size`, preserving the squircle.

    Edge-to-edge: used for Windows .ico and as input for macOS .icns when
    we DON'T want the AI master's natural padding (e.g., smaller sizes where
    macOS dock auto-pad doesn't apply).
    """
    return _ensure_rgba(img).resize((size, size), Image.LANCZOS)


def master_with_padding(icon: Image.Image, target: int) -> Image.Image:
    """Place the cropped squircle onto a transparent target×target canvas,
    preserving the AI master's natural ~12% padding.

    Why: macOS renders every dock tile by auto-adding its own visual padding
    on top of the PNG. If our PNG is already edge-to-edge (squircle filling
    the entire canvas), the auto padding stacks on top and the icon looks
    bigger than its neighbors in the dock. Keeping the AI master's natural
    ~24% inset lets macOS's auto padding produce a properly-sized icon.

    The squircle inside the final canvas occupies ~76% of the area, centered.
    """
    bg = Image.new("RGBA", (target, target), (0, 0, 0, 0))  # fully transparent
    side = int(target * 0.76)
    scaled = _ensure_rgba(icon).resize((side, side), Image.LANCZOS)
    offset = ((target - side) // 2, (target - side) // 2)
    bg.paste(scaled, offset, scaled)
    return bg


def write_png(img: Image.Image, path: Path) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    _ensure_rgba(img).save(path, "PNG", optimize=True)
    print(f"  -> {path.name} ({img.size[0]}x{img.size[1]})")


def write_ico(img: Image.Image, path: Path) -> None:
    # Windows .ico can embed multiple sizes; include 16/32/48/64/128/256.
    sizes = [(s, s) for s in (16, 24, 32, 48, 64, 128, 256)]
    path.parent.mkdir(parents=True, exist_ok=True)
    _ensure_rgba(img).save(path, format="ICO", sizes=sizes)
    print(f"  -> {path.name} (multi-size .ico)")


def write_icns(img: Image.Image, path: Path) -> None:
    """Build a macOS .icns via the Apple iconutil utility.

    iconutil requires a `*.iconset` directory of PNGs at canonical sizes and
    filenames. PIL's native icns writer only handles a subset, so we let
    macOS do the heavy lifting.
    """
    import shutil
    import subprocess

    # Stage the iconset under /tmp so we don't leave junk in the project tree.
    # iconutil REQUIRES the directory to end in `.iconset` (Apple convention).
    iconset_dir = Path("/tmp/bilusic.iconset")
    if iconset_dir.exists():
        shutil.rmtree(iconset_dir)
    iconset_dir.mkdir()

    apple_pairs = [
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
        (1024, "icon_1024x1024.png"),
    ]
    for s, name in apple_pairs:
        scaled = img if img.size[0] == s else square_to_size(img, s)
        # iconutil wraps raw PNG into icns — each input MUST be RGBA, so go
        # through _ensure_rgba for the same reason as the bundle PNGs.
        _ensure_rgba(scaled).save(iconset_dir / name, "PNG")

    path.parent.mkdir(parents=True, exist_ok=True)
    try:
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset_dir), "-o", str(path)],
            check=True,
        )
        print(f"  -> {path.name} (macOS .icns via iconutil, {len(apple_pairs)} sizes)")
    finally:
        shutil.rmtree(iconset_dir, ignore_errors=True)


def main() -> None:
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    print(f"Cropping {MASTER_PATH.name}...")
    icon = auto_crop_icon(MASTER_PATH)
    print(f"  cropped to {icon.size}")
    # Round the corners of the subject so the macOS dock renders a squircle
    # instead of a hard square (see apply_squircle_mask docstring).
    icon = apply_squircle_mask(icon)
    print(f"  squircle masked to {icon.size}")

    # All sizes preserve the AI master's ~12% padding around the squircle so
    # macOS dock doesn't auto-enlarge the icon by stacking its own visual
    # padding on top of an already-edge-to-edge image.
    write_png(master_with_padding(icon, 1024), OUT_DIR / "icon.png")
    print("Writing Tauri bundle icons...")
    write_png(master_with_padding(icon, 32), OUT_DIR / "32x32.png")
    write_png(master_with_padding(icon, 128), OUT_DIR / "128x128.png")
    write_png(master_with_padding(icon, 256), OUT_DIR / "128x128@2x.png")
    write_png(master_with_padding(icon, 256), OUT_DIR / "256x256.png")
    write_png(master_with_padding(icon, 512), OUT_DIR / "512x512.png")

    # macOS .icns and Windows .ico both can carry transparent padding, so we
    # give them the padded versions too — same logic as above for icns, and
    # harmless for ico (Windows doesn't auto-pad).
    print("Writing ico (Windows)...")
    write_ico(master_with_padding(icon, 256), OUT_DIR / "icon.ico")
    print("Writing icns (macOS)...")
    write_icns(master_with_padding(icon, 1024), OUT_DIR / "icon.icns")
    print("Done.")


if __name__ == "__main__":
    main()
