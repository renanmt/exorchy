#!/usr/bin/env python3
"""Build the bundled config archive for a language pack (SLP / PLP).

eXo ships every game's `dosbox.conf` inside the pack's metadata ZIP - 3.8 GB
for Spanish, 805 MB for Polish - under `eXo/eXoDOS/!dos/<langdir>/<code>/`.
Without it, a language-pack-exclusive game has no config to launch with: the
English catalog has no counterpart to borrow one from, which left 363 games
downloadable but unplayable.

Stripped to `.conf`/`.bat` the result is a few hundred KB, the same trade the
eXoDOS and GLP archives already make.

The per-game XML fragments (`xml/<langdir>/<code>`) carry the mapping from
eXo's launcher bat to its directory - the catalogue's own SLP/PLP XML has no
shortcode at all, so this is the only place that link exists. It is written
next to the archive as `<pack>_confdirs.txt` (`<bat stem>:<code>`), which
`generate_db` reads to fill `dosbox_conf`.

Usage:
    scripts/gen_lp_configs.py <metadata.zip> --lang '!spanish' --out-prefix SLP
"""

import argparse
import re
import sys
import zipfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
KEEP_SUFFIXES = (".conf", ".bat")


def game_dirs_and_confs(zf: zipfile.ZipFile, lang: str) -> tuple[list[str], set[str]]:
    """Config entries to keep, and the set of directories holding a dosbox.conf."""
    prefix = f"eXo/eXoDOS/!dos/{lang}/"
    keep: list[str] = []
    with_conf: set[str] = set()
    for name in zf.namelist():
        if not name.startswith(prefix) or name.endswith("/"):
            continue
        if not name.lower().endswith(KEEP_SUFFIXES):
            continue
        keep.append(name)
        rest = name[len(prefix):]
        if "/" in rest and rest.rsplit("/", 1)[1].lower() == "dosbox.conf":
            with_conf.add(rest.split("/", 1)[0])
    return keep, with_conf


"""Bats every game directory carries - they name no game."""
GENERIC_BATS = {"install.bat", "exception.bat", "alternate launcher.bat"}


def bat_to_dir(entries: list[str], lang: str) -> dict[str, str]:
    """Map each game's launcher bat stem to eXo's directory name.

    eXo names the launcher after the game (`PC Mus (1996).bat`), which is
    exactly what the catalogue's ApplicationPath carries - so the bat stem is
    the join key. Titles are NOT used: they collide across the catalog (three
    different DOS games are called "Fallout"), bat names carry the year.
    """
    prefix = f"eXo/eXoDOS/!dos/{lang}/"
    mapping: dict[str, str] = {}
    for name in entries:
        rest = name[len(prefix):]
        if "/" not in rest:
            continue
        code, file = rest.split("/", 1)
        if "/" in file or not file.lower().endswith(".bat"):
            continue
        if file.lower() in GENERIC_BATS:
            continue
        mapping[file[:-4]] = code
    return mapping


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("metadata_zip", type=Path)
    ap.add_argument("--lang", required=True, help="e.g. '!spanish'")
    ap.add_argument("--out-prefix", required=True, help="e.g. 'SLP'")
    args = ap.parse_args()

    if not args.metadata_zip.exists():
        print(f"error: {args.metadata_zip} not found", file=sys.stderr)
        return 1

    out_zip = REPO_ROOT / "metadata" / f"{args.out_prefix}_configs.zip"
    out_map = REPO_ROOT / "metadata" / f"{args.out_prefix}_confdirs.txt"

    with zipfile.ZipFile(args.metadata_zip) as zf:
        keep, with_conf = game_dirs_and_confs(zf, args.lang)
        if not keep:
            print(f"error: no config entries under {args.lang}", file=sys.stderr)
            return 1
        mapping = bat_to_dir(keep, args.lang)

        # Deterministic order so a rebuild without content changes is a no-op.
        with zipfile.ZipFile(out_zip, "w", zipfile.ZIP_DEFLATED) as out:
            for name in sorted(keep):
                out.writestr(name, zf.read(name))

    # Only games whose directory actually holds a config are worth recording.
    usable = {bat: code for bat, code in mapping.items() if code in with_conf}
    lines = [f"{bat}:{code}" for bat, code in sorted(usable.items())]
    out_map.write_text("\n".join(lines) + "\n", encoding="utf-8")

    print(f"{out_zip.name}: {len(keep)} files, {out_zip.stat().st_size / 1024:.0f} KB")
    print(f"{out_map.name}: {len(usable)} games mapped ({len(with_conf)} dirs with a config)")
    missing = sorted(set(with_conf) - set(usable.values()))
    if missing:
        print(f"  {len(missing)} config dirs without an XML fragment, e.g. {missing[:5]}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
