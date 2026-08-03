<!--
SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
SPDX-License-Identifier: MIT
-->
# `city_splat.py` — open-data city/route splats

Turns European open-data lidar (+ aerial imagery for color) into martin `.ply` Gaussian-splat
clouds and matching `[camera]` flight tracks. It exists because martin's other city content (the
Google Earth Studio aerial captures, see `AERIAL-CITIES.md`) is **local-only** — Google's terms
don't allow redistributing those captures, so nothing built from them can ship. Everything this
script fetches is open data (CC0 / CC-BY / dl-zero — see **Licensing** below), so the resulting
`.ply` files and any show built from them can be committed, bundled, and shown anywhere.

Started Netherlands-only (hence the `~/.cache/martin-nl-data` cache directory name — kept for
continuity, not worth a churn-inducing rename) and grew to also fetch German (North
Rhine-Westphalia) open geodata, including genuine cross-border routes that stitch both countries'
data into one shared coordinate frame.

## Quick start

```bash
# one-time setup — a venv with laspy (LAZ/LAS reading) + pyproj (cross-border reprojection)
python3 -m venv ~/.cache/martin-nl-data/venv
~/.cache/martin-nl-data/venv/bin/pip install laspy[laszip] numpy pyproj pillow

# a single city block
~/.cache/martin-nl-data/venv/bin/python pipeline/city_splat.py rotterdam

# a flight route, streamed as overlapping segments + a matching camera track
~/.cache/martin-nl-data/venv/bin/python pipeline/city_splat.py koeln \
    --segments 3 --count 6000000 --emit-camera
```

Output lands in `assets/cities/` (gitignored — regenerate any time; the `.show` files that
reference these `.ply`s are what's committed). Reference an entry directly in a `.show`:

```
[settings]
ply = assets/cities/rotterdam_tight.ply
...
```

## Licensing (why this is all shippable)

| Country | Height data | License | Imagery / color source | License |
|---|---|---|---|---|
| NL | AHN5/AHN4 lidar | **CC0** (data.overheid.nl dataset 47567) | GeoTiles (TU Delft) colorized redistribution, from Beeldmateriaal Nederland aerial imagery | **CC-BY 4.0** → credit "GeoTiles/TU Delft" |
| DE (NRW) | bDOM50 (colored raster surface) or 3dm laserscan (colorless) | **dl-zero** (CC0-class, no attribution legally required) | bDOM50's own baked RGB, or a separate 10 cm DOP orthophoto for 3dm | **dl-zero** |

No attribution is *legally* required for CC0/dl-zero, but crediting the source is the polite,
honest move — every shipped show has an on-screen `[caption] screentext:` credit line. Typical
wording:

- NL: `hoogtedata AHN (CC0) · kleur GeoTiles · TU Delft / Beeldmateriaal NL`
- DE: `Geodaten © Land NRW — open data`
- Cross-border route: both lines, one per country's data (see `productions/cities/grens.show`'s
  `[caption]` for a worked example — it credits height *and* imagery, both countries).

## The `CITIES` dict — four entry shapes

Everything is driven by one dict in the script. An entry is one of:

**1. A fixed city block** (`tiles` + `bbox`) — crops one or more GeoTiles subtiles to a tight
window so every city gets the same footprint (morphs pair cleanly):
```python
"rotterdam": {"tiles": ["37FZ1_23"], "bbox": (92000, 437500, 93000, 438750)},
```

**2. A flight route** (`route` + `width`) — a polyline the camera flies, cropped to a corridor;
subtiles under it are looked up automatically (no `tiles` needed):
```python
"denhaag-zee": {"route": [(82300, 455250), (81450, 455300), ...], "width": 700},
```
Add `"provider": "nrw3d"` to fetch NRW data instead of NL (German routes, plain UTM32 metres):
```python
"koeln": {"provider": "nrw3d", "width": 700, "route": [(356549, 5645283), ...]},
```

**3. A cross-border route** (`legs`, not `route`) — each leg keeps its own provider/native CRS,
reprojected to a shared UTM32 frame before the normal segment/camera machinery runs. Give a leg
`"latlon": True` to specify its waypoints as plain WGS84 (lat, lon) — handy for German via-points
looked up as town coordinates rather than surveyed RD/UTM:
```python
"grens": {
    "legs": [
        {"provider": None, "width": 500, "route": [(258074, 471388), (263000, 471000)]},
        {"provider": "nrw3d", "width": 500, "latlon": True, "route": [(52.207, 7.023), (52.14, 7.28)]},
    ],
},
```

**4. A flat area map** (`area`) — a huge bbox, not a corridor, meant to be flown over from height
as a "painted map" rather than a street-level scene:
```python
"amsterdam-map": {"area": (115500, 482000, 127500, 492000)},
```

## Adding a new city or route

1. **Find coordinates.** For a city block: look up the GeoTiles subtile name(s) covering it — the
   [PDOK kaartbladindex](https://service.pdok.nl/rws/actueel-hoogtebestand-nederland/atom/downloads/dsm_05m/kaartbladindex.json)
   (cached automatically to `~/.cache/martin-nl-data/kaartbladindex.json`) maps sheet names to RD
   New (EPSG:28992) bboxes. For a route: RD New waypoints (a few meters of precision is plenty —
   the corridor `width` provides slack) via the
   [PDOK Locatieserver](https://api.pdok.nl/bzk/locatieserver/search/v3_1/free) or by eyeballing
   coordinates against a RD-New basemap. For a German route: UTM32 (EPSG:25832) waypoints, or use
   `"latlon": True` with plain lat/lon town coordinates and skip the reprojection step yourself.
2. **Dry-run the tile count first.** Before fetching anything, sanity-check how much data a new
   route touches — `route_tiles`/`grid_tiles` are pure functions you can call from a REPL, or just
   read the printed `"→ N subtiles"` / `"-> N tiles"` line at the start of a real run and Ctrl-C if
   it's a surprise (a 24 km flight can touch 60+ subtiles; a bad waypoint typo can 10x that). Each
   NL subtile is ~200-280 MB, each NRW 1 km² tile ~50-120 MB — a wildly oversized route will show
   up as a wildly oversized tile count before you've downloaded anything.
3. **Add the `CITIES` entry**, run it, eyeball the printed point count and the `.ply` file size.
4. **For a route:** add `--segments N --emit-camera` once the single-cloud version looks right —
   this streams the corridor as overlapping arc-length segments (see **Long routes** below) and
   prints a ready-to-paste `[camera]` track in the same coordinate frame.

## Long routes: `--segments` streaming

A single `.ply` can't hold an arbitrarily long corridor under the GPU's resident-splat budget.
`--segments N` splits it into `N` overlapping arc-length bins, each its own `.ply`, all sharing
**one** coordinate transform (computed over the whole corridor) — so a `pair=match` morph can swap
segment *k* for *k+1* **in place** mid-flight: the overlap zone barely moves, the tail streams out,
the nose streams in. This needs `[settings] normalize = 0` in the `.show` (the engine's normal
per-part re-centering would break the shared frame and misalign every seam) and `pair = match`.
`--emit-camera` prints a matching `[camera]` track in the same transform.

`--rainbow-seg K` bakes a rainbow hue-along-the-arc into segment `K`'s own colors at generation
time (`koeln_seg1_rainbow_tight.ply` is one of these) — this is **not** the same thing as the
runtime `raster:position` `.show` modifier, which scatters splat *positions* by a hash (a confetti
effect) rather than recoloring them; don't reach for `raster:position` expecting a rainbow-terrain
look on a plain (non-pre-baked) segment, it doesn't do that.

## CLI reference

| Flag | Default | Meaning |
|---|---|---|
| `city` | — | A `CITIES` key, or `all` (fixed city blocks only — skips route/legs entries) |
| `--count` | 1200000 | Splats in the output (per-segment, if `--segments` is used) |
| `--out` | `assets/cities` | Output directory |
| `--scale-mult` | 1.4 | Splat radius = mult × mean point spacing (coverage vs crispness) |
| `--opacity` | 0.85 | Per-splat opacity |
| `--seed` | 1 | Subsample RNG seed |
| `--emit-camera` | off | Route/legs entries: print a `[camera]` track flying the route |
| `--duration` | 80.0 | Flight seconds for `--emit-camera` / segment timing |
| `--segments` | 0 | Split a route into N overlapping shared-frame segments |
| `--seg-overlap` | 0.30 | Overlap per seam, as a fraction of one segment's length |
| `--tile-cap` | 2500000 | Per-tile point intake cap (RAM guard for long routes — raise it for short max-density routes) |
| `--rainbow-seg` | -1 | Bake rainbow colors into segment K (see above) |
| `--dist-m` / `--pitch` / `--alt-m` | 260 / 0.30 / 35 | `--emit-camera` orbit distance / downward pitch / target height, in **meters** |

## Output format

Binary sh0 `.ply`, byte-identical to `src/splatgen.rs::write_ply` — 14 `f32` per point: xyz,
log-scale ×3, logit-opacity, an identity quaternion, `f_dc = (rgb - 0.5) / 0.2820948`. Files are
authored **Y-DOWN**; martin's loader applies a 180°-about-X flip on load, so after that flip: world
x = east, y = up, z = south-negative (i.e. z = -north). `emit_camera`'s track uses the identical
mapping, so a generated camera always lands in the same world as its cloud.

## Known limits / troubleshooting

- **RAM.** A long route can touch 60+ tiles; `--tile-cap` caps how many points come in from any
  ONE tile before final subsampling (raising it only helps short high-density routes — for long
  ones it just burns RAM for points that get thrown away at the final `--count` subsample anyway).
- **Missing tiles.** Both `fetch_any` (NL, AHN5→AHN4 fallback) and the cross-border `load_mixed_route`
  path skip a tile that 404s on every source tried rather than aborting the whole run — expected at
  the edges of coverage (a border-adjacent NL subtile, or an NRW grid cell just past the state
  line). A single-provider `load_route` run still hard-exits on a missing tile — that's usually a
  real problem (wrong coordinates), not an edge case.
- **Cache.** Every downloaded tile is cached under `~/.cache/martin-nl-data/` (LAZ/LAS files, the
  kaartbladindex, NRW DOP JPEGs) — delete it to force a re-fetch, or just let it grow (re-running
  the same city/route is then instant).
- **Stale segments.** `write_segments` deletes any `{name}_seg*` file in the output dir that the
  current run didn't just write — an older run's segments (different `--count`/`--segments`, a
  slightly different shared transform) would otherwise silently mix with the new set and misalign
  at the seams.
