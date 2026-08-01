#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
# SPDX-License-Identifier: MIT
"""NL open-data city → martin sh0 splat .ply — the SHIPPABLE replacement for the Google aerial
captures (which are local-only, see pipeline/AERIAL-CITIES.md). One command per city:

    ~/.cache/martin-nl-data/venv/bin/python pipeline/nl_city_splat.py rotterdam
    ... amsterdam / denhaag / utrecht      (--count 1200000, --out austin_run/exports/)

Source: GeoTiles (TU Delft) AHN5 subtiles — 1×1.25 km cuts of the national lidar, ALREADY COLORED
with the national aerial imagery (RGB in the LAZ). Data licensing (all open, all shippable):
  * AHN lidar points: CC0 (data.overheid.nl dataset 47567)
  * GeoTiles colorized redistribution: CC-BY 4.0 → credit "GeoTiles/TU Delft"
  * point colors derive from Beeldmateriaal Nederland aerial imagery (open data)
Show credit line: "hoogtedata AHN (CC0) · kleur GeoTiles · TU Delft / Beeldmateriaal NL".

Output matches martin's sh0 .ply layout exactly (src/splatgen.rs::write_ply — 14 floats/point:
xyz, log-scale ×3, logit-opacity, identity quat, f_dc = (rgb-0.5)/0.2820948) and the capture
orientation convention: the file is Y-DOWN (martin rotates every .ply 180° about X on load), so
we write y = -height. World after the flip: x=east, y=up, z=-north.

LAZ facts (verified on the Rotterdam Markthal subtile): point format 8, RGB stored 0-255 in the
uint16 fields (divide by 255, NOT 65535); classes 1=unclassified (trees/cars) 2=ground 6=building
9=water 26=bridges; EPSG:28992 (RD New) metres + NAP heights; ~25-35M points per subtile.
"""
import argparse
import os
import sys
import urllib.request
from pathlib import Path

import numpy as np

try:
    import laspy
except ImportError:
    sys.exit("laspy missing — run: ~/.cache/martin-nl-data/venv/bin/python (see module docstring)")

CACHE = Path.home() / ".cache" / "martin-nl-data"
GEOTILES = "https://geotiles.citg.tudelft.nl/{src}/{tile}.LAZ"

# City centres — GeoTiles subtile(s) + the RD bbox to CROP to (subtiles carry a 20 m overlap rim;
# a tight ~1×1.25 km window keeps every city the same footprint → morphs pair cleanly). A bbox may
# span multiple subtiles (they merge). `src` defaults to AHN5_T; AHN5 was only flown for the
# Randstad by 2023, so northern cities fall back to AHN4_T (GeoTiles colorizes both, and AHN4 is
# actually denser). Subtile URLs verified 2026-08-01 (sizes 239-281 MB each).
CITIES = {
    # Markthal / Blaak / Laurenskerk + the Maas high-rises south of Blaak.
    "rotterdam": {"tiles": ["37FZ1_23"], "bbox": (92000, 437500, 93000, 438750)},
    # Dam / canal ring west of the Amstel.
    "amsterdam": {"tiles": ["25GN1_02"], "bbox": (121000, 486250, 122000, 487500)},
    # Centraal Station + Hoftoren/ministry towers + Binnenhof/Hofvijver + Malieveld — spans the
    # subtile-02/03 column seam, so two downloads merge into the one window.
    "denhaag": {"tiles": ["30GZ1_02", "30GZ1_03"], "bbox": (81400, 455000, 82400, 456250)},
    # Domtoren / old centre.
    "utrecht": {"tiles": ["31HZ2_02"], "bbox": (136000, 455000, 137000, 456250)},
    # Grote Markt / Martinitoren — deFEEST's home turf (tile via the PDOK kaartbladindex; AHN4:
    # no AHN5 flight for Groningen yet).
    "groningen": {"tiles": ["07DN1_24"], "bbox": (233000, 581250, 234000, 582500), "src": "AHN4_T"},
    # FLYOVER STRIP, not a city block: the Rotterdam Maas axis — Euromast → Erasmusbrug → Kop van
    # Zuid/Noordereiland, 3.5×1.25 km across four subtiles. Same mechanics, elongated bbox: the
    # normalizer just yields a long thin cloud the camera can fly along. Use a bigger --count
    # (points spread over 3.5× the area) but stay under the ~2M GPU record cap.
    "maas": {"tiles": ["37HN1_01", "37HN1_02", "37HN1_03", "37HN1_04"],
             "bbox": (90500, 436250, 94000, 437500)},
}

SH_C0 = 0.282_094_8  # SH degree-0 basis constant (matches splatgen.rs)


def fetch(tile: str, src: str) -> Path:
    """Download one GeoTiles subtile into the cache (skip when present)."""
    CACHE.mkdir(parents=True, exist_ok=True)
    dst = CACHE / f"{src}-{tile}.LAZ"
    if dst.exists() and dst.stat().st_size > 1_000_000:
        return dst
    url = GEOTILES.format(src=src, tile=tile)
    print(f"  fetch {url}")
    tmp = dst.with_suffix(".part")
    with urllib.request.urlopen(url) as r, open(tmp, "wb") as f:
        while chunk := r.read(1 << 20):
            f.write(chunk)
    os.replace(tmp, dst)
    print(f"  cached {dst} ({dst.stat().st_size / 1e6:.0f} MB)")
    return dst


def load_city(name: str) -> tuple[np.ndarray, np.ndarray]:
    """Read + crop + merge the city's subtiles → (xyz float64 RD/NAP, rgb float32 0..1)."""
    spec = CITIES[name]
    x0, y0, x1, y1 = spec["bbox"]
    src = spec.get("src", "AHN5_T")
    pts, cols = [], []
    for tile in spec["tiles"]:
        las = laspy.read(fetch(tile, src))
        if "red" not in las.point_format.dimension_names:
            sys.exit(f"{tile}: no RGB dimensions — expected a GeoTiles colorized subtile")
        x, y, z = np.asarray(las.x), np.asarray(las.y), np.asarray(las.z)
        keep = (x >= x0) & (x < x1) & (y >= y0) & (y < y1)
        # Drop water returns (class 9, ~0.02%): lidar barely sees water, the stray returns are
        # noise specks. Everything else stays — ground, buildings, and the unclassified bucket
        # (trees, cars, street furniture) that makes a city read as LIVED-IN, like the captures.
        keep &= np.asarray(las.classification) != 9
        pts.append(np.column_stack([x[keep], y[keep], z[keep]]))
        # GeoTiles stores 0-255 in the uint16 RGB fields (verified) — /255, not /65535.
        rgb = np.column_stack(
            [np.asarray(las.red)[keep], np.asarray(las.green)[keep], np.asarray(las.blue)[keep]]
        ).astype(np.float32) / 255.0
        cols.append(rgb)
    xyz = np.concatenate(pts)
    rgb = np.clip(np.concatenate(cols), 0.0, 1.0)
    print(f"  {name}: {len(xyz):,} points in bbox after crop")
    return xyz, rgb


def to_martin(xyz: np.ndarray, count: int, seed: int) -> tuple[np.ndarray, float]:
    """RD/NAP → martin file coords: centre, normalize 1/p90 (the capture convention), Y-DOWN
    (martin's load flip makes it upright), subsample to `count`. Returns (pos f32, spacing)."""
    rng = np.random.default_rng(seed)
    if len(xyz) > count:
        idx = rng.choice(len(xyz), size=count, replace=False)
    else:
        idx = np.arange(len(xyz))
    p = xyz[idx]
    cx, cy = (p[:, 0].min() + p[:, 0].max()) / 2, (p[:, 1].min() + p[:, 1].max()) / 2
    ground = np.percentile(p[:, 2], 5)  # maaiveld anchor, robust to basements/outliers
    east, north, up = p[:, 0] - cx, p[:, 1] - cy, p[:, 2] - ground
    # Normalize so the ground footprint's p90 radius lands at 1.0 — same scale the Brush captures
    # arrive at, so the existing show's camera distances/orbits keep working.
    r90 = np.percentile(np.hypot(east, north), 90)
    s = 1.0 / r90
    # File is Y-DOWN: y = -up. z = north → world z = -north after the 180° X flip.
    pos = np.column_stack([east * s, -up * s, north * s]).astype(np.float32)
    # Mean point spacing of the dominant (ground) plane, for the splat radius: the cropped
    # footprint is (x1-x0)×(y1-y0) m² holding `count` points → spacing = sqrt(area/count), scaled.
    area = (p[:, 0].max() - p[:, 0].min()) * (p[:, 1].max() - p[:, 1].min())
    spacing = float(np.sqrt(area / len(p)) * s)
    return pos, spacing, idx


def write_ply(path: Path, name: str, pos: np.ndarray, rgb: np.ndarray, scale: float, opacity: float):
    """martin's sh0 binary .ply — byte-identical layout to src/splatgen.rs::write_ply."""
    header = (
        f"ply\nformat binary_little_endian 1.0\ncomment martin NL-city splat: {name} "
        f"(AHN CC0 · kleur GeoTiles CC-BY TU Delft)\n"
        f"element vertex {len(pos)}\n"
        "property float x\nproperty float y\nproperty float z\n"
        "property float scale_0\nproperty float scale_1\nproperty float scale_2\n"
        "property float opacity\n"
        "property float rot_0\nproperty float rot_1\nproperty float rot_2\nproperty float rot_3\n"
        "property float f_dc_0\nproperty float f_dc_1\nproperty float f_dc_2\nend_header\n"
    )
    n = len(pos)
    rec = np.empty((n, 14), dtype="<f4")
    rec[:, 0:3] = pos
    rec[:, 3:6] = np.log(scale)
    rec[:, 6] = np.log(opacity / (1.0 - opacity))
    rec[:, 7] = 1.0
    rec[:, 8:11] = 0.0
    rec[:, 11:14] = (rgb - 0.5) / SH_C0
    with open(path, "wb") as f:
        f.write(header.encode())
        f.write(rec.tobytes())
    print(f"  wrote {path} ({n:,} splats, radius {scale:.5f}, opacity {opacity})")


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("city", choices=sorted(CITIES) + ["all"])
    ap.add_argument("--count", type=int, default=1_200_000, help="splats per city")
    ap.add_argument("--out", default="austin_run/exports", help="output dir (the show's asset root)")
    ap.add_argument("--scale-mult", type=float, default=1.4,
                    help="splat radius = mult × mean point spacing (coverage vs crispness)")
    ap.add_argument("--opacity", type=float, default=0.85)
    ap.add_argument("--seed", type=int, default=1)
    a = ap.parse_args()
    outdir = Path(a.out)
    outdir.mkdir(parents=True, exist_ok=True)
    for name in sorted(CITIES) if a.city == "all" else [a.city]:
        print(f"{name}:")
        xyz, rgb = load_city(name)
        pos, spacing, idx = to_martin(xyz, a.count, a.seed)
        write_ply(outdir / f"{name}_tight.ply", name, pos, rgb[idx], a.scale_mult * spacing, a.opacity)


if __name__ == "__main__":
    main()
