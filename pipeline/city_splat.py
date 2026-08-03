#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
# SPDX-License-Identifier: MIT
"""European open-data city/route → martin sh0 splat .ply — the SHIPPABLE replacement for the
Google aerial captures (which are local-only, see pipeline/AERIAL-CITIES.md). Started NL-only
(hence the historical `martin-nl-data` cache dir name below), now also fetches German (NRW) open
data and can stitch a route across the NL/DE border in one shared frame. Full reference,
worked examples, and a from-scratch walkthrough for adding a new city/route: pipeline/CITY-SPLAT.md
— read that first if you're new here. One command per city/route:

    ~/.cache/martin-nl-data/venv/bin/python pipeline/city_splat.py rotterdam
    ~/.cache/martin-nl-data/venv/bin/python pipeline/city_splat.py koeln --segments 3 --emit-camera
    ... amsterdam / denhaag / utrecht / grens / ...   (--count 1200000, --out assets/cities/)

Data sources + licensing (all open, all shippable — see CITY-SPLAT.md for the full attribution
table and per-provider details):
  NL   GeoTiles (TU Delft) AHN5/AHN4 subtiles — lidar CC0 (data.overheid.nl dataset 47567),
       colorized-redistribution CC-BY 4.0 ("GeoTiles/TU Delft"), colors from Beeldmateriaal NL.
  DE   NRW open geodata (opengeodata.nrw.de) — bDOM50 (colored raster surface) or 3dm lidar
       (colorless, true point structure) + a DOP orthophoto for color — both dl-zero (CC0-class,
       no attribution required; "Geodaten © Land NRW" is the polite courtesy line anyway).
Typical show credit lines: "hoogtedata AHN (CC0) · kleur GeoTiles · TU Delft / Beeldmateriaal NL"
(NL) and "Geodaten © Land NRW — open data" (DE).

Output matches martin's sh0 .ply layout exactly (src/splatgen.rs::write_ply — 14 floats/point:
xyz, log-scale ×3, logit-opacity, identity quat, f_dc = (rgb-0.5)/0.2820948) and the capture
orientation convention: the file is Y-DOWN (martin rotates every .ply 180° about X on load), so
the FILE gets y = -height and z = +north; the 180° X flip negates BOTH → world: x = east,
y = up, z = -north (south-positive). The camera emit uses the same mapping (z = -(ry-cy)*s).
Geography check: denhaag-zee starts SOUTH of the route centre and lands at world z = +0.77.

LAZ facts (verified on the Rotterdam Markthal subtile): point format 8, RGB stored 0-255 in the
uint16 fields (divide by 255, NOT 65535); classes 1=unclassified (trees/cars) 2=ground 6=building
9=water 26=bridges; EPSG:28992 (RD New) metres + NAP heights; ~25-35M points per subtile. NRW LAZ
is EPSG:25832 (UTM32) metres — see CITY-SPLAT.md for its own per-provider point-format notes.
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

# Only needed for `legs` (cross-border) route entries — imported lazily inside load_mixed_route so
# a bare NL-only run never needs it installed.

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
    # THE MAP — a huge flat AREA, not a corridor: 12×10 km of Amsterdam and surroundings
    # (Sloterdijk ↔ IJburg, Zuidas ↔ Waterland). Fly HIGH: at map counts the splats are ~15 m
    # blobs — a living painted map, not a street-level scene.
    "amsterdam-map": {"area": (115500, 482000, 127500, 492000)},
    # ULTRA — the canal-ring core at SOURCE-density: 3×2.5 km (Jordaan ↔ Artis, Leidseplein ↔ the
    # IJ) at 16M splats ≈ 1 m splat pitch, the maximum this data gives. Fly LOW (the houses are
    # only ~20 m tall): ~80 m camera height reads as a drone shot over the canals.
    "amsterdam-ultra": {"area": (119800, 485800, 122800, 488300)},
    # KÖLN — the EVOKE greeting: Dom → across the Rhine over the Deutzer Brücke → Deutz → KALK,
    # ending on the ABENTEUERHALLEN (the party venue itself). German open data: NRW bDOM50 tiles
    # (RGB from 2024/25 aerial imagery, EPSG:25832 = plain UTM32 meters, license dl-zero = CC0
    # class, no attribution required). Coordinates geocoded via Nominatim → UTM32.
    "koeln": {"provider": "nrw3d", "width": 700,
              "route": [(356549, 5645283), (356800, 5645000), (357100, 5644723),
                        (357700, 5644650), (358800, 5644560), (360088, 5644507)]},
    # THE BORDER CROSSING — a real national border, real data both sides, one continuous flight:
    # Enschede (Oude Markt, NL) → the actual Dutch/German border → Gronau → ~20 km into NRW
    # (Ochtrup direction). ~24 km total. `legs` (not `route`): each leg keeps its OWN provider/CRS
    # (NL = RD New/AHN via GeoTiles, DE = UTM32/NRW 3dm+DOP), reprojected to a shared UTM32 frame
    # by `load_mixed_route` before the normal arc-length/segment machinery runs — this pipeline's
    # first cross-border route. Dry-run tile count (route_tiles/grid_tiles, no downloads) BEFORE
    # committing: 18 NL subtiles (~4.7 GB) + 68 DE grid cells (~4 GB) ≈ 9 GB total — sane; the
    # original Amsterdam→Enschede→Köln full epic dry-ran at 427+672 tiles (~150+ GB) and was cut.
    "grens": {
        "legs": [
            {"provider": None, "width": 500,
             "route": [(258074, 471388), (263000, 471000)]},        # Enschede -> toward the border
            {"provider": "nrw3d", "width": 500, "latlon": True,
             "route": [(52.207, 7.023), (52.14, 7.28)]},             # Gronau -> ~20 km into NRW
        ],
    },
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


def bbox_tiles(x0, y0, x1, y1):
    """All GeoTiles subtiles whose 1×1.25 km cell overlaps the bbox — the AREA mode (a huge flat
    map instead of a route corridor)."""
    tiles = []
    for name, bx0, by0, bx1, by1 in _load_kaartbladen():
        if bx1 < x0 or bx0 > x1 or by1 < y0 or by0 > y1:
            continue
        for sub in range(1, 26):
            col, row = (sub - 1) % 5, (sub - 1) // 5
            sx0, sy1 = bx0 + col * 1000, by1 - row * 1250
            if sx0 + 1000 < x0 or sx0 > x1 or sy1 < y0 or sy1 - 1250 > y1:
                continue
            tiles.append(f"{name}_{sub:02d}")
    return tiles


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


NRW_BDOM = ("https://www.opengeodata.nrw.de/produkte/geobasis/hm/bdom50_las/bdom50_las/"
            "bdom50_32{e}_{n}_1_nw_{year}.laz")


def fetch_nrw(e_km: int, n_km: int) -> Path:
    """Fetch one NRW bDOM50 tile (1×1 km, EPSG:25832, LAS PDRF 2 = RGB from the 2024/25 aerial
    imagery — the German sibling of the GeoTiles colorized subtiles; license dl-zero-de/2.0, no
    attribution required). The year suffix varies by region → try recent years."""
    CACHE.mkdir(parents=True, exist_ok=True)
    dst = CACHE / f"bdom50_{e_km}_{n_km}.laz"
    if dst.exists() and dst.stat().st_size > 100_000:
        return dst
    last = None
    for year in (2025, 2024, 2023, 2022):
        url = NRW_BDOM.format(e=e_km, n=n_km, year=year)
        try:
            print(f"  fetch {url}")
            tmp = dst.with_suffix(".part")
            with urllib.request.urlopen(url) as r, open(tmp, "wb") as f:
                while chunk := r.read(1 << 20):
                    f.write(chunk)
            os.replace(tmp, dst)
            print(f"  cached {dst} ({dst.stat().st_size / 1e6:.0f} MB)")
            return dst
        except urllib.error.HTTPError as err:
            last = err
            if err.code != 404:
                raise
    sys.exit(f"bdom50 {e_km}_{n_km}: no year variant found ({last})")


NRW_3DM = ("https://www.opengeodata.nrw.de/produkte/geobasis/hm/3dm_l_las/3dm_l_las/"
           "3dm_32_{e}_{n}_1_nw.laz")
NRW_DOP_WMS = ("https://www.wms.nrw.de/geobasis/wms_nw_dop?SERVICE=WMS&VERSION=1.3.0"
               "&REQUEST=GetMap&LAYERS=nw_dop_rgb&STYLES=&CRS=EPSG:25832"
               "&BBOX={x0},{y0},{x1},{y1}&WIDTH={px}&HEIGHT={px}&FORMAT=image/jpeg")
DOP_PX = 4000  # 4000 px over 1 km = 25 cm — plenty to color 9.5 pts/m² lidar (33 cm spacing)


def fetch_nrw_3dm(e_km: int, n_km: int) -> Path:
    """Fetch one NRW 3dm laserscan tile (1×1 km, ~9.5 pts/m², NO RGB — LAS PDRF 1). The real
    lidar: true point structure (see-through trees, sharp edges) vs the bDOM's melted 0.5 m
    raster surface. Colored separately from the DOP orthophoto (fetch_nrw_dop)."""
    CACHE.mkdir(parents=True, exist_ok=True)
    dst = CACHE / f"3dm_{e_km}_{n_km}.laz"
    if dst.exists() and dst.stat().st_size > 100_000:
        return dst
    url = NRW_3DM.format(e=e_km, n=n_km)
    print(f"  fetch {url}")
    tmp = dst.with_suffix(".part")
    with urllib.request.urlopen(url) as r, open(tmp, "wb") as f:
        while chunk := r.read(1 << 20):
            f.write(chunk)
    os.replace(tmp, dst)
    print(f"  cached {dst} ({dst.stat().st_size / 1e6:.0f} MB)")
    return dst


def fetch_nrw_dop(e_km: int, n_km: int) -> np.ndarray:
    """One km-tile of the NRW 10 cm orthophoto via WMS, as an RGB array (DOP_PX², 25 cm/px) —
    the color source for the colorless 3dm lidar. Cached as JPEG."""
    from PIL import Image

    CACHE.mkdir(parents=True, exist_ok=True)
    dst = CACHE / f"dop_{e_km}_{n_km}.jpg"
    if not dst.exists() or dst.stat().st_size < 10_000:
        url = NRW_DOP_WMS.format(x0=e_km * 1000, y0=n_km * 1000,
                                 x1=e_km * 1000 + 1000, y1=n_km * 1000 + 1000, px=DOP_PX)
        print(f"  fetch DOP {e_km}_{n_km} (WMS)")
        tmp = dst.with_suffix(".part")
        with urllib.request.urlopen(url) as r, open(tmp, "wb") as f:
            f.write(r.read())
        os.replace(tmp, dst)
    return np.asarray(Image.open(dst).convert("RGB"))


def color_from_dop(x, y, e_km, n_km, dop):
    """Sample per-point RGB (0..1 f32) from a tile's DOP array. WMS row 0 = NORTH edge."""
    res = 1000.0 / DOP_PX
    px = np.clip(((x - e_km * 1000) / res).astype(np.int32), 0, DOP_PX - 1)
    py = np.clip(((n_km * 1000 + 1000 - y) / res).astype(np.int32), 0, DOP_PX - 1)
    return dop[py, px].astype(np.float32) / 255.0


def grid_tiles(route=None, width=0, area=None):
    """1×1 km UTM32 grid cells under a route corridor or area bbox → [(e_km, n_km)] — the NRW
    tile scheme needs no sheet index, the file name IS the grid coordinate."""
    if area:
        x0, y0, x1, y1 = area
    else:
        rx = [p[0] for p in route]
        ry = [p[1] for p in route]
        m = width / 2
        x0, y0, x1, y1 = min(rx) - m, min(ry) - m, max(rx) + m, max(ry) + m
    cells = []
    for e in range(int(x0 // 1000), int(x1 // 1000) + 1):
        for n in range(int(y0 // 1000), int(y1 // 1000) + 1):
            if route is not None:
                cx, cy = e * 1000 + 500, n * 1000 + 500
                sx = np.array([cx, e * 1000.0, e * 1000 + 1000.0, e * 1000.0, e * 1000 + 1000.0])
                sy = np.array([cy, n * 1000.0, n * 1000.0, n * 1000 + 1250.0, n * 1000 + 1000.0])
                if _dist_to_polyline(sx, sy, route).min() > width / 2 + 750:
                    continue
            cells.append((e, n))
    return cells


def fetch_any(tile: str) -> Path:
    """Fetch a subtile, AHN5 first, AHN4 fallback (AHN5 only covers the Randstad so far)."""
    for src in ("AHN5_T", "AHN4_T"):
        try:
            return fetch(tile, src)
        except urllib.error.HTTPError as e:
            if e.code != 404:
                raise
    sys.exit(f"{tile}: not on GeoTiles in AHN5_T or AHN4_T")


args_tile_cap = [2_500_000]  # set from --tile-cap in main


def load_route(name: str) -> tuple[np.ndarray, np.ndarray, list]:
    """Read every subtile under the corridor, crop to it → (xyz, rgb, route)."""
    spec = CITIES[name]
    route, width = spec["route"], spec["width"]
    provider = spec.get("provider")
    nrw = provider in ("nrw", "nrw3d")
    tiles = spec.get("tiles") or (
        grid_tiles(route=route, width=width) if nrw else route_tiles(route, width)
    )
    print(f"  route: {len(route)} waypoints, corridor {width} m → {len(tiles)} subtiles: {tiles}")
    pts, cols = [], []
    for tile in tiles:
        if provider == "nrw3d":
            # FUSION: the real 3dm lidar (9.5 pts/m², colorless) colored per-point from the 10 cm
            # DOP orthophoto — 2.4× the density of the bDOM raster AND true point structure.
            las = laspy.read(fetch_nrw_3dm(*tile))
            dop = fetch_nrw_dop(*tile)
        else:
            las = laspy.read(fetch_nrw(*tile) if nrw else fetch_any(tile))
            if "red" not in las.point_format.dimension_names:
                sys.exit(f"{tile}: no RGB dimensions — expected a colorized tile")
            dop = None
        x, y, z = np.asarray(las.x), np.asarray(las.y), np.asarray(las.z)
        keep = _dist_to_polyline(x, y, route) <= width / 2
        keep &= np.asarray(las.classification) != 9
        if not keep.any():
            continue
        sel = np.flatnonzero(keep)
        # RAM guard for LONG routes (a 55 km flight touches 60+ tiles on a 27 GB box): cap the
        # per-tile intake — 2.5M points/tile is still ~3× any realistic end budget's share.
        # (4M×60 tiles + laspy's own per-tile decompress peak OOM-killed the adam-denhaag run.)
        if len(sel) > args_tile_cap[0]:
            sel = np.random.default_rng(len(sel)).choice(sel, size=args_tile_cap[0], replace=False)
        # float32 positions: RD coords are ~1e5 m → f32 keeps ~1 cm relative precision, half the RAM.
        pts.append(np.column_stack([x[sel], y[sel], z[sel]]).astype(np.float32))
        if dop is not None:
            cols.append(color_from_dop(x[sel], y[sel], tile[0], tile[1], dop))
        else:
            # GeoTiles stores 0-255 in the uint16 RGB fields; NRW bDOM uses full 16-bit — detect.
            cscale = 65535.0 if int(np.max(las.red[:10000])) > 255 else 255.0
            rgb = np.column_stack(
                [np.asarray(las.red)[sel], np.asarray(las.green)[sel], np.asarray(las.blue)[sel]]
            ).astype(np.float32) / cscale
            cols.append(rgb)
    xyz = np.concatenate(pts)
    rgb = np.clip(np.concatenate(cols), 0.0, 1.0)
    print(f"  {name}: {len(xyz):,} points in corridor")
    return xyz, rgb, route


def load_mixed_route(name: str) -> tuple[np.ndarray, np.ndarray, list]:
    """A CROSS-BORDER route (`legs`, not `route`): each leg keeps its own provider/native CRS for
    fetching (RD New/AHN via GeoTiles for NL legs, UTM32/NRW for DE legs — the tile-lookup and
    per-point crop logic is identical to load_route, just looped per leg), then every leg's POINTS
    and WAYPOINTS are reprojected into one shared UTM32 frame via pyproj before concatenating —
    downstream (write_segments/emit_camera/to_martin) never needs to know this was multi-provider,
    it just sees one big route+cloud. A `latlon: True` leg gives its route as (lat, lon) pairs
    (handy for German via-points looked up as plain WGS84 town coordinates, not surveyed RD/UTM)."""
    import pyproj

    spec = CITIES[name]
    to_utm_from_rd = pyproj.Transformer.from_crs("EPSG:28992", "EPSG:25832", always_xy=True)
    to_utm_from_wgs = pyproj.Transformer.from_crs("EPSG:4326", "EPSG:25832", always_xy=True)
    pts, cols, full_route = [], [], []
    for leg in spec["legs"]:
        provider = leg.get("provider")
        nrw = provider in ("nrw", "nrw3d")
        width = leg["width"]
        raw_route = leg["route"]
        # the route polyline itself, reprojected to UTM32 for the shared arc-length track
        if leg.get("latlon"):
            utm_route = [to_utm_from_wgs.transform(lon, lat) for lat, lon in raw_route]
        elif nrw:
            utm_route = list(raw_route)
        else:
            utm_route = [to_utm_from_rd.transform(x, y) for x, y in raw_route]
        full_route.extend(utm_route)
        # las points are native-CRS per leg: RD New for NL, UTM32 for NRW — the crop/tile-lookup
        # route must match THAT, not raw_route (which is lat/lon for a `latlon: True` leg).
        crop_route = utm_route if nrw else raw_route
        tiles = grid_tiles(route=crop_route, width=width) if nrw else route_tiles(raw_route, width)
        print(f"  leg ({provider or 'nl'}): {len(raw_route)} waypoints, corridor {width} m "
              f"-> {len(tiles)} tiles")
        for tile in tiles:
            if provider == "nrw3d":
                # grid-edge NRW tiles (e.g. right at the political border) can 404 too — skip, don't abort.
                try:
                    las = laspy.read(fetch_nrw_3dm(*tile))
                    dop = fetch_nrw_dop(*tile)
                except urllib.error.HTTPError as e:
                    print(f"  skip {tile}: {e}")
                    continue
            else:
                # border-edge tiles can genuinely miss coverage (fetch_any hard-exits on a 404 in
                # both AHN vintages) — skip and warn instead of killing the whole cross-border run.
                try:
                    las = laspy.read(fetch_nrw(*tile) if nrw else fetch_any(tile))
                except SystemExit as e:
                    print(f"  skip {tile}: {e}")
                    continue
                dop = None
                if "red" not in las.point_format.dimension_names:
                    print(f"  skip {tile}: no RGB dimensions — expected a colorized tile")
                    continue
            x, y, z = np.asarray(las.x), np.asarray(las.y), np.asarray(las.z)
            keep = _dist_to_polyline(x, y, crop_route) <= width / 2
            keep &= np.asarray(las.classification) != 9
            if not keep.any():
                continue
            sel = np.flatnonzero(keep)
            if len(sel) > args_tile_cap[0]:
                sel = np.random.default_rng(len(sel)).choice(sel, size=args_tile_cap[0], replace=False)
            xs, ys, zs = x[sel], y[sel], z[sel]
            # reproject THIS LEG'S points into the shared UTM32 frame (no-op for NRW legs — already there)
            if nrw:
                ux, uy = xs, ys
            else:
                ux, uy = to_utm_from_rd.transform(xs, ys)
            pts.append(np.column_stack([ux, uy, zs]).astype(np.float32))
            if dop is not None:
                cols.append(color_from_dop(xs, ys, tile[0], tile[1], dop))
            else:
                cscale = 65535.0 if int(np.max(las.red[:10000])) > 255 else 255.0
                rgb = np.column_stack(
                    [np.asarray(las.red)[sel], np.asarray(las.green)[sel], np.asarray(las.blue)[sel]]
                ).astype(np.float32) / cscale
                cols.append(rgb)
    xyz = np.concatenate(pts)
    rgb = np.clip(np.concatenate(cols), 0.0, 1.0)
    print(f"  {name}: {len(xyz):,} points across {len(spec['legs'])} legs (shared UTM32 frame)")
    return xyz, rgb, full_route


def _hsv_to_rgb_np(hsv):
    """Vectorized HSV→RGB (float 0..1 arrays) — colorsys is per-tuple, useless at 2M points."""
    h, s, v = hsv[:, 0] * 6.0, hsv[:, 1], hsv[:, 2]
    i = np.floor(h).astype(np.int32) % 6
    f = h - np.floor(h)
    p, q, t = v * (1 - s), v * (1 - s * f), v * (1 - s * (1 - f))
    r = np.choose(i, [v, q, p, p, t, v])
    g = np.choose(i, [t, v, v, q, p, p])
    b = np.choose(i, [p, p, t, v, v, q])
    return np.stack([r, g, b], axis=1).astype(np.float32)


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
        tag = ""
        if args.rainbow_seg == k:
            # RAINBOW variant: hue runs along the route arc (with a height shimmer), value keeps
            # the true-color luminance so the city structure stays readable. A pair=match seam
            # morph then FADES the real colors into the rainbow per splat — the recolor IS the
            # transition. (raster:position was a white-out here: a flat low route cloud has
            # almost no position spread to color by.)
            frac = (arc[m] - a) / max(b - a, 1e-9)
            lum = seg_rgb.mean(axis=1)
            hue = (frac * 3.0 - seg_pos[:, 1] * 8.0) % 1.0
            hsv = np.stack([hue, np.full_like(hue, 0.9), 0.35 + 0.65 * lum], axis=1)
            seg_rgb = _hsv_to_rgb_np(hsv)
            tag = "_rainbow"
        fname = f"{name.replace('-', '_')}_seg{k}{tag}_tight.ply"
        write_ply(outdir / fname, f"{name} seg{k}{tag}", seg_pos, seg_rgb,
                  args.scale_mult * spacing, args.opacity)
        print(f"splat:{fname}  @{hold:.0f},{morph:.0f},0  ~morph  backdrop:stars")
    print("# --- end generated reel ---")
    # Shared-frame guard: segments from an OLDER run (different --count/--segments → a slightly
    # different normalize frame) silently mix with the new set and misalign at the seams. Remove
    # any {name}_seg* file we did not just write.
    written = {f"{name.replace('-', '_')}_seg{k}{'_rainbow' if args.rainbow_seg == k else ''}_tight.ply"
               for k in range(n)}
    for old in outdir.glob(f"{name.replace('-', '_')}_seg*_tight.ply"):
        if old.name not in written:
            print(f"  WARN: removing stale segment from an older run: {old.name}")
            old.unlink()
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


def load_city(name: str, per_tile_cap: int = 0) -> tuple[np.ndarray, np.ndarray]:
    """Read + crop + merge the city's subtiles → (xyz f32 RD/NAP, rgb f32 0..1). `per_tile_cap`
    randomly thins each tile's intake — REQUIRED for big AREA maps (117 full tiles ≈ 3 BILLION
    points uncapped: the amsterdam-map run OOM-died collecting them; capping at ~4× the end
    budget's per-tile share loses nothing the final subsample would keep anyway)."""
    spec = CITIES[name]
    x0, y0, x1, y1 = spec["bbox"]
    src = spec.get("src", "AHN5_T")
    pts, cols = [], []
    for tile in spec["tiles"]:
        # honour an explicit per-city src; otherwise AHN5 with AHN4 fallback (big AREA bboxes can
        # brush against sheets the AHN5 flights haven't covered yet).
        las = laspy.read(fetch(tile, src) if "src" in spec else fetch_any(tile))
        if "red" not in las.point_format.dimension_names:
            sys.exit(f"{tile}: no RGB dimensions — expected a GeoTiles colorized subtile")
        x, y, z = np.asarray(las.x), np.asarray(las.y), np.asarray(las.z)
        keep = (x >= x0) & (x < x1) & (y >= y0) & (y < y1)
        # Drop water returns (class 9, ~0.02%): lidar barely sees water, the stray returns are
        # noise specks. Everything else stays — ground, buildings, and the unclassified bucket
        # (trees, cars, street furniture) that makes a city read as LIVED-IN, like the captures.
        keep &= np.asarray(las.classification) != 9
        sel = np.flatnonzero(keep)
        if per_tile_cap and len(sel) > per_tile_cap:
            sel = np.random.default_rng(len(sel)).choice(sel, size=per_tile_cap, replace=False)
        pts.append(np.column_stack([x[sel], y[sel], z[sel]]).astype(np.float32))
        # GeoTiles stores 0-255 in the uint16 RGB fields (verified) — /255, not /65535.
        rgb = np.column_stack(
            [np.asarray(las.red)[sel], np.asarray(las.green)[sel], np.asarray(las.blue)[sel]]
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
        f"ply\nformat binary_little_endian 1.0\ncomment martin open-data city splat: {name} "
        f"(pipeline/city_splat.py — see CITY-SPLAT.md for per-source licensing)\n"
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
    ap.add_argument("--out", default="assets/cities", help="output dir (the shows' asset root; gitignored)")
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
    ap.add_argument("--tile-cap", type=int, default=2_500_000,
                    help="per-tile point intake cap (RAM guard; raise for short max-density routes)")
    ap.add_argument("--rainbow-seg", type=int, default=-1,
                    help="bake RAINBOW colors (hue along the arc) into segment K — the pair=match "
                         "seam morphs then fade real color ↔ rainbow per splat")
    ap.add_argument("--dist-m", type=float, default=260.0, help="--emit-camera height/orbit distance in METERS")
    ap.add_argument("--pitch", type=float, default=0.30,
                    help="--emit-camera downward pitch (more land, less sky)")
    ap.add_argument("--alt-m", type=float, default=35.0, help="--emit-camera target height in METERS")
    a = ap.parse_args()
    args_tile_cap[0] = a.tile_cap
    outdir = Path(a.out)
    outdir.mkdir(parents=True, exist_ok=True)
    names = (sorted(n for n in CITIES if "route" not in CITIES[n] and "legs" not in CITIES[n])
             if a.city == "all" else [a.city])
    for name in names:
        print(f"{name}:")
        route = None
        if "area" in CITIES[name]:
            x0, y0, x1, y1 = CITIES[name]["area"]
            CITIES[name]["tiles"] = bbox_tiles(x0, y0, x1, y1)
            CITIES[name]["bbox"] = CITIES[name]["area"]
            print(f"  area {x1 - x0}×{y1 - y0} m → {len(CITIES[name]['tiles'])} subtiles")
            cap = max(200_000, 4 * a.count // max(1, len(CITIES[name]["tiles"])))
            xyz, rgb = load_city(name, per_tile_cap=cap)
        elif "legs" in CITIES[name]:
            xyz, rgb, route = load_mixed_route(name)
        elif "route" in CITIES[name]:
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
