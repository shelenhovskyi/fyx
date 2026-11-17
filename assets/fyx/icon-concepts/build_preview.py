"""Builds preview.png (usage: python build_preview.py <this dir> <scratch dir>) with ImageMagick: every concept at 512, 64, 32 and
16 px on a light and a dark background, plus 16 and 32 px magnified 4x (nearest neighbour) so the
small sizes can be judged pixel by pixel."""

import pathlib
import subprocess
import sys

concepts_dir = pathlib.Path(sys.argv[1])
work = pathlib.Path(sys.argv[2])
work.mkdir(parents=True, exist_ok=True)

CONCEPTS = [
    ("a", "A  tree F"),
    ("b", "B  fix mark"),
    ("c", "C  slot"),
]
SIZES = [512, 64, 32, 16]
MAGNIFIED = [16, 32]
BACKGROUNDS = [("light", "#F4F4F5", "#18181B"), ("dark", "#18181B", "#E4E4E7")]
GAP = 48
LABEL_HEIGHT = 56


def magick(*args):
    subprocess.run(["magick", *map(str, args)], check=True)


def render(concept, size):
    master = work / f"{concept}_1024.png"
    if not master.exists():
        magick("-background", "none", "-density", 96, concepts_dir / f"{concept}.svg",
               "-resize", "1024x1024", "+repage", master)
    # Small sizes (Windows .ico frames, Linux 16-64 px) use the tile without the macOS margin.
    source = master
    if size <= 64:
        source = work / f"{concept}_tile.png"
        if not source.exists():
            magick(master, "-crop", "824x824+100+100", "+repage", source)
    out = work / f"{concept}_{size}.png"
    magick(source, "-filter", "Lanczos", "-resize", f"{size}x{size}", "+repage", out)
    return out


def panel(concept, title, background, text_color):
    tiles = []
    for size in SIZES:
        tiles.append((render(concept, size), f"{size} px"))
    for size in MAGNIFIED:
        magnified = work / f"{concept}_{size}_x4.png"
        magick(render(concept, size), "-filter", "point", "-resize", f"{size * 4}x{size * 4}",
               "+repage", magnified)
        tiles.append((magnified, f"{size} px x4"))

    width = GAP + sum(int(subprocess.run(["magick", "identify", "-format", "%w", str(path)],
                                         capture_output=True, text=True, check=True).stdout) + GAP
                      for path, _ in tiles)
    height = LABEL_HEIGHT + 512 + LABEL_HEIGHT
    out = work / f"panel_{concept}_{background.strip('#')}.png"
    args = ["-size", f"{width}x{height}", f"xc:{background}",
            "-font", "Arial", "-fill", text_color, "-pointsize", "30",
            "-annotate", f"+{GAP}+40", title,
            "-pointsize", "20", "-annotate", f"+{GAP + 560}+40", "64 px and smaller: tile without the macOS margin"]
    x = GAP
    baseline = LABEL_HEIGHT + 512
    for path, label in tiles:
        tile_size = int(subprocess.run(["magick", "identify", "-format", "%w", str(path)],
                                       capture_output=True, text=True, check=True).stdout)
        args += [str(path), "-geometry", f"+{x}+{baseline - tile_size}", "-composite",
                 "-pointsize", "20", "-annotate", f"+{x}+{baseline + 34}", label]
        x += tile_size + GAP
    magick(*args, out)
    return out


rows = []
for concept, title in CONCEPTS:
    panels = [panel(concept, title, background, text) for _, background, text in BACKGROUNDS]
    row = work / f"row_{concept}.png"
    magick(*panels, "+append", "+repage", row)
    rows.append(row)

magick(*rows, "-append", "+repage", concepts_dir / "preview.png")
print(concepts_dir / "preview.png")
