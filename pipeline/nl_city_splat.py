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
    # FLIGHT ROUTE — a polyline the camera flies, cropped to a corridor. No `tiles`: the subtiles
    # under the corridor are looked up from the cached PDOK kaartbladindex, AHN5→AHN4 fallback per
    # tile. `--emit-camera` writes the matching `[camera]` waypoints (same normalize transform).
    # Den Haag: CS → Binnenhof/Hofvijver → a SNAKE through the Zeeheldenkwartier → the dunes →
    # Scheveningen Pier — the city flows in, winds, and back OUT to the sea.
    "denhaag-zee": {"route": [(82300, 455250), (81450, 455300), (81100, 455900),
                              (80450, 455750), (80250, 456650), (79700, 456950),
                              (79950, 457750), (79350, 458350)],
                    "width": 700},
    # THE LONG ONE — the Randstad flight: Rotterdam Markthal → out over the northwest of town →
    # Delft (Markt/Oude Kerk) → Rijswijk → Den Haag CS/Binnenhof → the dunes → Scheveningen.
    # ~24 km; meant for --segments 8+ (the streamed mode) — never as one cloud.
    "randstad": {"route": [(92500, 437450), (91500, 439000), (89800, 441500),
                           (87500, 444500), (84350, 447550), (83800, 449800),
                           (83500, 451800), (82800, 453800), (82300, 455250),
                           (81450, 455300), (80600, 456300), (79950, 457750),
                           (79350, 458350)],
                 "width": 500},
    # Amsterdam → Den Haag (~59 km): LANGE VONDER (het Schelvischhoofd, Amsterdam-Noord, geocoded
    # via the PDOK Locatieserver) → across the IJ past CS → Dam → the canal ring → SCHIPHOL (over
    # the runways) → the Kagerplassen → Leiden (Pieterskerk) → Voorschoten → the HOFTOREN
    # (Rijnstraat 8, Den Haag). Meant for --segments 13 --count 700000 (seam-resident 1.4M).
    "adam-denhaag": {"route": [(121367, 493054), (121450, 490900), (121700, 488600),
                               (121400, 487200), (120900, 485600), (118500, 483000),
                               (114500, 480800), (111400, 479500), (108000, 476500),
                               (105000, 473000), (100500, 468500), (96500, 465500),
                               (93550, 463650), (91500, 461500), (89400, 459700),
                               (86000, 457200), (83000, 455600), (82087, 455163)],
                     "width": 800},
}


def _load_kaartbladen():
    """Cached PDOK kaartbladindex → [(name, x0, y0, x1, y1)] per 5×6.25 km AHN sheet."""
    import json

    idx = CACHE / "kaartbladindex.json"
    if not idx.exists():
        url = ("https://service.pdok.nl/rws/actueel-hoogtebestand-nederland/atom/downloads/"
               "dsm_05m/kaartbladindex.json")
        with urllib.request.urlopen(url) as r:
            idx.write_bytes(r.read())
    out = []
    for f in json.loads(idx.read_text())["features"]:
        g = f["geometry"]
        ring = g["coordinates"][0] if g["type"] == "Polygon" else g["coordinates"][0][0]
        xs = [c[0] for c in ring]
        ys = [c[1] for c in ring]
        name = f["properties"]["kaartbladNr"].removeprefix("R_")
        out.append((name, min(xs), min(ys), max(xs), max(ys)))
    return out


def _dist_to_polyline(px, py, route, want_arc=False):
    """Min distance of points (px, py arrays) to the route polyline — vectorized per segment.
    With want_arc, also returns each point's ARC LENGTH along the route at its closest approach
    (what the segment splitter bins on)."""
    best = np.full(len(px), np.inf)
    arc = np.zeros(len(px)) if want_arc else None
    acc = 0.0
    for (ax, ay), (bx, by) in zip(route, route[1:]):
        dx, dy = bx - ax, by - ay
        L = float(np.hypot(dx, dy))
        L2 = max(L * L, 1e-9)
        t = np.clip(((px - ax) * dx + (py - ay) * dy) / L2, 0.0, 1.0)
        d = np.hypot(px - (ax + t * dx), py - (ay + t * dy))
        if want_arc:
            closer = d < best
            arc[closer] = acc + t[closer] * L
        np.minimum(best, d, out=best)
        acc += L
    return (best, arc) if want_arc else best


def route_tiles(route, width):
    """All GeoTiles subtiles whose 1×1.25 km cell comes within width/2 of the route polyline."""
    tiles = []
    margin = width / 2
    for name, bx0, by0, bx1, by1 in _load_kaartbladen():
        rx = [p[0] for p in route]
        ry = [p[1] for p in route]
        if bx1 < min(rx) - margin or bx0 > max(rx) + margin:
            continue
        if by1 < min(ry) - margin or by0 > max(ry) + margin:
            continue
        for sub in range(1, 26):
            col, row = (sub - 1) % 5, (sub - 1) // 5
            sx0, sy1 = bx0 + col * 1000, by1 - row * 1250
            # cell-to-polyline distance via the cell's centre + corner sample (cheap, safe margin)
            cx, cy = sx0 + 500, sy1 - 625
            samples_x = np.array([cx, sx0, sx0 + 1000, sx0, sx0 + 1000], dtype=float)
            samples_y = np.array([cy, sy1, sy1, sy1 - 1250, sy1 - 1250], dtype=float)
            if _dist_to_polyline(samples_x, samples_y, route).min() <= margin + 800:
                tiles.append(f"{name}_{sub:02d}")
    return tiles


def fetch_any(tile: str) -> Path:
    """Fetch a subtile, AHN5 first, AHN4 fallback (AHN5 only covers the Randstad so far)."""
    for src in ("AHN5_T", "AHN4_T"):
        try:
            return fetch(tile, src)
        except urllib.error.HTTPError as e:
            if e.code != 404:
                raise
    sys.exit(f"{tile}: not on GeoTiles in AHN5_T or AHN4_T")


def load_route(name: str) -> tuple[np.ndarray, np.ndarray, list]:
    """Read every subtile under the corridor, crop to it → (xyz, rgb, route)."""
    spec = CITIES[name]
    route, width = spec["route"], spec["width"]
    tiles = spec.get("tiles") or route_tiles(route, width)
    print(f"  route: {len(route)} waypoints, corridor {width} m → {len(tiles)} subtiles: {tiles}")
    pts, cols = [], []
    for tile in tiles:
        las = laspy.read(fetch_any(tile))
        if "red" not in las.point_format.dimension_names:
            sys.exit(f"{tile}: no RGB dimensions — expected a GeoTiles colorized subtile")
        x, y, z = np.asarray(las.x), np.asarray(las.y), np.asarray(las.z)
        keep = _dist_to_polyline(x, y, route) <= width / 2
        keep &= np.asarray(las.classification) != 9
        if not keep.any():
            continue
        sel = np.flatnonzero(keep)
        # RAM guard for LONG routes (a 55 km flight touches 60+ tiles on a 27 GB box): cap the
        # per-tile intake — 2.5M points/tile is still ~3× any realistic end budget's share.
        # (4M×60 tiles + laspy's own per-tile decompress peak OOM-killed the adam-denhaag run.)
        if len(sel) > 2_500_000:
            sel = np.random.default_rng(len(sel)).choice(sel, size=2_500_000, replace=False)
        # float32 positions: RD coords are ~1e5 m → f32 keeps ~1 cm relative precision, half the RAM.
        pts.append(np.column_stack([x[sel], y[sel], z[sel]]).astype(np.float32))
        rgb = np.column_stack(
            [np.asarray(las.red)[sel], np.asarray(las.green)[sel], np.asarray(las.blue)[sel]]
        ).astype(np.float32) / 255.0
        cols.append(rgb)
    xyz = np.concatenate(pts)
    rgb = np.clip(np.concatenate(cols), 0.0, 1.0)
    print(f"  {name}: {len(xyz):,} points in corridor")
    return xyz, rgb, route


def write_segments(name, xyz, rgb, route, args, outdir):
    """Split a route corridor into overlapping ARC-LENGTH segments — the 'tiles stream in and out'
    mode. Every segment shares ONE transform (computed over the full corridor subsample), so a
    `pair=match` morph swaps segment k for k+1 IN PLACE mid-flight: the overlap zone barely moves
    (same points on both sides of the seam), the tail streams out, the nose streams in. Play with
    `[settings] normalize = 0` — the engine's per-part re-normalization would break the shared frame.
    Prints the [reel] block to paste."""
    n = args.segments
    pos_all, spacing, idx, (cx, cy, ground, s) = to_martin(xyz, args.count * n, args.seed)
    rgb_all = rgb[idx]
    px, py = xyz[idx][:, 0], xyz[idx][:, 1]
    _, arc = _dist_to_polyline(px, py, route, want_arc=True)
    total = arc.max()
    seg_len = total / n
    ov = args.seg_overlap * seg_len
    print(f"  segments: {n} × {seg_len:.0f} m (+{ov:.0f} m overlap each side), shared frame")
    print("\n# --- generated [reel] (paste into the .show; needs [settings] normalize = 0, pair = match) ---")
    print("[reel]")
    # Per segment slot = duration/n, split hold/morph. The morph wants ~12% of the flight but never
    # more than 60% of its own slot (many segments → the seams dominate; a 0-hold slot broke here).
    seg_t = args.duration / n
    morph = min(args.duration * 0.12, seg_t * 0.6)
    hold = max(seg_t - morph, 1.0)
    for k in range(n):
        a, b = k * seg_len - ov, (k + 1) * seg_len + ov
        m = (arc >= a) & (arc < b)
        seg_pos, seg_rgb = pos_all[m], rgb_all[m]
        fname = f"{name.replace('-', '_')}_seg{k}_tight.ply"
        write_ply(outdir / fname, f"{name} seg{k}", seg_pos, seg_rgb,
                  args.scale_mult * spacing, args.opacity)
        print(f"splat:{fname}  @{hold:.0f},{morph:.0f},0  ~morph  backdrop:stars")
    print("# --- end generated reel ---")
    return cx, cy, ground, s


def emit_camera(route, cx, cy, ground, s, duration, dist_m=260.0, pitch=0.30, alt_m=35.0):
    """Print a `[camera]` track flying the route in the SAME normalize transform as the cloud:
    martin file coords (x=east, y=-up, z=north) become world (east, up, -north) after the load
    flip, so a route point (rx, ry) targets world (x=(rx-cx)*s, y=alt, z=-(ry-cy)*s). Times
    follow arc length. Yaw follows the flight direction.
    `dist_m`/`alt_m` are METERS — normalized units depend on route length (a 24 km route
    normalizes ~10× smaller than a city block, which made a fixed `dist=0.28` fly km-high on
    long routes); the generator multiplies by the route's own scale. Defaults fly CLOSE with a
    downward pitch — more land, less sky.
    NOTE: no whitespace padding in `t=` — a padded `t=  0.0` silently parses UNTIMED and the
    whole track is ignored in favour of the auto-frame."""
    dist = dist_m * s
    alt = alt_m * s
    seg = [np.hypot(bx - ax, by - ay) for (ax, ay), (bx, by) in zip(route, route[1:])]
    total = sum(seg)
    t, acc = [], 0.0
    for d in [0.0, *seg]:
        acc += d
        t.append(acc / total * duration)
    print("\n# --- generated flight track (paste into the .show) ---")
    print("[camera]")
    for i, ((rx, ry), tt) in enumerate(zip(route, t)):
        x, z = (rx - cx) * s, -(ry - cy) * s
        # look along the path: direction to the next waypoint (last one keeps the previous heading).
        # martin's orbit cam sits at target + dist·(cos yaw, ·, sin yaw) LOOKING BACK at the target,
        # so flying forward means yaw = atan2(dz,dx) + π (calibrated on the denhaag-zee stills).
        j = min(i, len(route) - 2)
        dx, dz = (route[j + 1][0] - route[j][0]) * s, -(route[j + 1][1] - route[j][1]) * s
        yaw = float((np.arctan2(dz, dx) + 2.0 * np.pi) % (2.0 * np.pi) - np.pi)
        print(f"t={tt:.1f}  pos={x:.4f},{alt:.4f},{z:.4f}  dist={dist:.4f}  "
              f"yaw={yaw:.2f}  pitch={pitch:.2f}")
    print("# --- end generated track ---\n")

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


def to_martin(xyz: np.ndarray, count: int, seed: int):
    """RD/NAP → martin file coords: centre, normalize 1/p90 (the capture convention), Y-DOWN
    (martin's load flip makes it upright), subsample to `count`.
    Returns (pos f32, spacing, idx, transform (cx, cy, ground, s)) — the transform is what
    `emit_camera` reuses so a flight track lands in the same world as the cloud."""
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
    return pos, spacing, idx, (float(cx), float(cy), float(ground), float(s))


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
    ap.add_argument("--emit-camera", action="store_true",
                    help="route entries: print a [camera] track flying the route (same transform)")
    ap.add_argument("--duration", type=float, default=80.0,
                    help="flight time in seconds for --emit-camera / segment timing")
    ap.add_argument("--segments", type=int, default=0,
                    help="route entries: split into N overlapping arc-length segments in ONE shared "
                         "frame (play with [settings] normalize = 0 + pair = match); --count is "
                         "PER SEGMENT")
    ap.add_argument("--seg-overlap", type=float, default=0.30,
                    help="overlap per seam as a fraction of segment length")
    ap.add_argument("--dist-m", type=float, default=260.0, help="--emit-camera height/orbit distance in METERS")
    ap.add_argument("--pitch", type=float, default=0.30,
                    help="--emit-camera downward pitch (more land, less sky)")
    ap.add_argument("--alt-m", type=float, default=35.0, help="--emit-camera target height in METERS")
    a = ap.parse_args()
    outdir = Path(a.out)
    outdir.mkdir(parents=True, exist_ok=True)
    names = sorted(n for n in CITIES if "route" not in CITIES[n]) if a.city == "all" else [a.city]
    for name in names:
        print(f"{name}:")
        route = None
        if "route" in CITIES[name]:
            xyz, rgb, route = load_route(name)
        else:
            xyz, rgb = load_city(name)
        if route is not None and a.segments > 0:
            cx, cy, ground, s = write_segments(name, xyz, rgb, route, a, outdir)
        else:
            pos, spacing, idx, (cx, cy, ground, s) = to_martin(xyz, a.count, a.seed)
            fname = name.replace("-", "_")
            write_ply(outdir / f"{fname}_tight.ply", name, pos, rgb[idx],
                      a.scale_mult * spacing, a.opacity)
        if route is not None and a.emit_camera:
            emit_camera(route, cx, cy, ground, s, a.duration,
                        dist_m=a.dist_m, pitch=a.pitch, alt_m=a.alt_m)


if __name__ == "__main__":
    main()
