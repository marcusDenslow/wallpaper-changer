"""Makes copies of the Schibsted Grotesk instances with tabular figures baked in.

egui does no OpenType shaping, so `font-variant-numeric: tabular-nums` from style.css
can't be applied at draw time. This remaps the cmap to the `tnum` alternates instead.

    pip install fonttools
    python tools/freeze_tnum.py   (from native/)

Writes assets/sans-<weight>-tnum.ttf for the weights the CSS used tabular-nums at.
"""
from pathlib import Path

from fontTools.ttLib import TTFont

ASSETS = Path(__file__).resolve().parent.parent / "assets"
WEIGHTS = ["450", "500", "600"]


def single_subst(lookup):
    for sub in lookup.SubTable:
        if lookup.LookupType == 7:  # extension
            sub = sub.ExtSubTable
        if sub.LookupType != 1:
            raise SystemExit(f"tnum lookup type {sub.LookupType} isn't a single substitution")
        yield from sub.mapping.items()


def freeze(src: Path, dst: Path) -> int:
    font = TTFont(src)
    gsub = font["GSUB"].table
    indices = {i for rec in gsub.FeatureList.FeatureRecord if rec.FeatureTag == "tnum" for i in rec.Feature.LookupListIndex}
    mapping = {}
    for i in sorted(indices):
        mapping.update(single_subst(gsub.LookupList.Lookup[i]))
    if not mapping:
        raise SystemExit(f"{src.name} has no tnum feature")
    for table in font["cmap"].tables:
        table.cmap = {cp: mapping.get(glyph, glyph) for cp, glyph in table.cmap.items()}
    font.save(dst)
    return len(mapping)


for w in WEIGHTS:
    n = freeze(ASSETS / f"sans-{w}.ttf", ASSETS / f"sans-{w}-tnum.ttf")
    print(f"sans-{w}-tnum.ttf: {n} glyphs remapped")
