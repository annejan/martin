---
name: render
description: >-
  Render a martin .show / production to an mp4 the safe way — low-quality preview first, OOM- and
  disk-aware full render, then verify + deliver + clean up. Invoke whenever asked to render, record,
  or "make a video / mp4" of a show, or to produce contact-shot stills for vetting a scene.
---

# Rendering a martin show to mp4

The hard-won workflow. Skipping steps wastes a long GPU render or fills the disk.

## 1. Vet CHEAP before a full render
Never go straight to 1080p60. First either:
- **Contact stills** (fastest): `MARTIN_RES=640x360 MARTIN_MUTE=1 MARTIN_SHOT=/tmp/s.png MARTIN_SHOTS=12,27,52 ./target/release/martin <show>` — seeks to those seconds, dumps PNGs. Read them to check composition/placement.
- **Low-Q preview clip**: `MARTIN_RES=480x270 MARTIN_PREVIEW_FPS=8 MARTIN_MORPH_COUNT=200000`.

Vet placement GPU-free with `pipeline/show_layout.py <show>` (top-down + screen-projection + overlap check) — don't guess 3D coords blind.

## 2. Full render
```
TMPDIR=/home/annejan/.cache/martin-render \
  MARTIN_RES=1920x1080 MARTIN_PREVIEW_FPS=60 \
  MARTIN_SHOW=<show> ./record.sh /home/annejan/Videos/<name>_1080p60.mp4
```
- **`TMPDIR` MUST be a real disk**, not `/tmp` (RAM tmpfs → "Disk quota exceeded" mid-render). `record.sh` now disk-pre-flights and aborts early if it won't fit.
- Run it **in the background** (renders are minutes long; you're re-invoked on completion).
- Render **one at a time** — concurrent renders wedge the iGPU.

## 3. If it's over budget (the GPU-budget trap)
The 860M OOMs ~2.5M resident gaussians. A `--record` over the `MARTIN_SPLAT_WARN` soft cap (default 2M)
now **fails fast, before any frames are written**: an `ERROR: … refusing to start a long dump` and
`exit 1` — not a mid-render death. (Live playback just logs a `WARN` and plays on.) Fix: lower
**`MARTIN_MORPH_COUNT`** for the record only (e.g. `=70000`) — the `.show` `budget` stays high for live
play. Explosive `~entrance` parts (explode/shockwave/shatter/vortex/implode) each add an origin cloud,
so they push the count up fast.

## 4. Verify
```
ffprobe -v error -show_entries format=duration:stream=width,height,nb_frames \
  -of default=noprint_wrappers=1 <out.mp4>
```
- `moov atom not found` / `Invalid data` ⇒ **ffmpeg is still muxing** — wait, re-check (don't deliver a half-written file).
- check `nb_frames ≈ duration × fps`, and `grep -c panic` the log (a teardown panic AFTER all frames is benign — the mp4 is complete).

## 5. Deliver + clean
- **Deliver with `SendUserFile`** — `Read` only shows the file to you, not the user (they'll see nothing).
- Clean scratch after: `rm -rf /home/annejan/.cache/martin-render/tmp.*`.

## Notes
- Recording runs **headless** (offscreen Image, no window) — works over SSH / with `DISPLAY` unset.
- `--validate` is a dry run (prints the resolved timeline + exits) — it does NOT build the scene, so the GPU-budget / disk warnings don't fire there; they fire on a real run.
