#!/usr/bin/env python3
"""
Rebuild the bundled Tier 0 preview set from the full-size covers.

`gen_thumbnails.py` derives each preview from the 400 px cover it just wrote,
so the previews are a pure function of `thumbnails/<collection>/` - no metadata
ZIP needed. That matters twice:

  * `thumbnails/eXoDOS` is filled by FOUR passes (eXoDOS + GLP/SLP/PLP, see
    init-dev.sh). Re-running only the eXoDOS pass leaves every LP-sourced cover
    without a preview, and the LP zips are 25 GB.
  * Changing the preview size otherwise means re-decoding 36k source images.

Usage:
    python3 scripts/gen_previews.py [<collection> ...]     (default: all)

Dependencies: Pillow
"""

import sys
from pathlib import Path

from PIL import Image

REPO = Path(__file__).resolve().parent.parent
THUMBS = REPO / "thumbnails"
PREVIEWS = REPO / "src-tauri/resources/previews"

# Keep in sync with gen_thumbnails.py's preview block - the grid tile is 165 px,
# so 120 px upscales by 1.4x and stays legible.
WIDTH, QUALITY = 120, 55


def rebuild(collection: str) -> None:
    src, dst = THUMBS / collection, PREVIEWS / collection
    if not src.is_dir():
        sys.exit(f"No covers at {src} - run init-dev first")
    dst.mkdir(parents=True, exist_ok=True)
    covers = sorted(src.glob("*.jpg"))
    for cover in covers:
        img = Image.open(cover).convert("RGB")
        height = max(1, int(img.height * WIDTH / img.width))
        img.resize((WIDTH, height), Image.LANCZOS).save(
            dst / cover.name, "JPEG", quality=QUALITY, optimize=True
        )
    # Covers can disappear between runs (a title dropped from the catalogue);
    # a stale preview would then outlive the art it was made from.
    stale = {p.name for p in dst.glob("*.jpg")} - {c.name for c in covers}
    for name in stale:
        (dst / name).unlink()
    total = sum(p.stat().st_size for p in dst.glob("*.jpg"))
    print(f"{collection}: {len(covers)} previews, {total / 1048576:.1f} MB"
          + (f", {len(stale)} stale removed" if stale else ""))


def main() -> None:
    names = sys.argv[1:] or sorted(p.name for p in THUMBS.iterdir() if p.is_dir())
    for name in names:
        rebuild(name)


if __name__ == "__main__":
    main()
