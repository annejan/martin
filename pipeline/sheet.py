#!/usr/bin/env python3
"""Contact-sheet storyboard for a martin show — one thumbnail per shot in a grid, so you can read a
show's whole arc at a glance without sitting through a full render.

How it works (no engine changes — pure tooling over the existing headless capture):
  1. run the show with MARTIN_VALIDATE=1 to dump the resolved cue timeline (each shot's start time),
  2. for each shot, run a headless MARTIN_SHOT screenshot at that shot's *mid-hold* moment,
  3. downscale each frame and composite them into one grid PNG (Pillow).

Usage:
    pipeline/sheet.py [out.png]              # uses $MARTIN_SHOW (or set MARTIN_* as usual)
    MARTIN_SHOW=productions/cities/cities-defeest.show pipeline/sheet.py /tmp/cities-sheet.png

Honours every MARTIN_* env var you set (MARTIN_SHOW / MARTIN_SEQ / MARTIN_RES / …). Deterministic:
each thumbnail is a frame-indexed MARTIN_SHOT, so the sheet is reproducible.
"""
import os
import re
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(HERE, "target", "release", "martin")
# A shot line in the MARTIN_VALIDATE dump: e.g.  "  [02] t=  40.0s @@  mesh defeest.glb ~Shockwave …"
SHOT_RE = re.compile(r"\[(\d+)\]\s+t=\s*([\d.]+)s\s+(.*)")


def env_base():
    e = dict(os.environ)
    e.setdefault("BEVY_ASSET_ROOT", HERE)  # same as record.sh, so basenames resolve
    e["MARTIN_FFT"] = "0"  # skip the spectrum bake — thumbnails don't need it (faster)
    return e


def shot_times():
    """(start_time, label) per shot, from the MARTIN_VALIDATE dump."""
    e = env_base()
    e["MARTIN_VALIDATE"] = "1"
    out = subprocess.run([BIN], env=e, capture_output=True, text=True, timeout=180).stdout
    shots = []
    for line in out.splitlines():
        m = SHOT_RE.search(line)
        if m:
            shots.append((float(m.group(2)), m.group(3).strip()[:40]))
    return shots


def shot_tag(at):
    """Match the engine's shot_path() suffix (capture.rs): `_<t>` with the 1-dp value, `.`/`-`→`_`."""
    return f"_{round(at * 10) / 10:g}".replace(".", "_").replace("-", "_")


def capture_all(times, stem):
    """Capture EVERY time in ONE engine run (MARTIN_SHOTS) — amortizes the cold start (~4x faster than
    a run per shot). `stem` like <td>/f.png → returns the per-time output paths."""
    e = env_base()
    e["MARTIN_SHOT"] = stem
    e["MARTIN_SHOTS"] = ",".join(f"{t:.3f}" for t in times)
    subprocess.run([BIN], env=e, capture_output=True, text=True, timeout=600)
    base, ext = stem.rsplit(".", 1)
    return [f"{base}{shot_tag(t)}.{ext}" for t in times]


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "martin-sheet.png"
    if not os.path.exists(BIN):
        sys.exit(f"build first: cargo build --release  (missing {BIN})")
    if "MARTIN_SHOW" not in os.environ and "MARTIN_SEQ" not in os.environ:
        sys.exit("set MARTIN_SHOW=<show> (or MARTIN_SEQ) first")
    try:
        from PIL import Image, ImageDraw
    except ImportError:
        sys.exit("needs Pillow:  pip install pillow")

    shots = shot_times()
    if not shots:
        sys.exit("no shots found in the MARTIN_VALIDATE dump")
    print(f"{len(shots)} shots — capturing thumbnails …")

    # each shot's mid-hold: halfway to the next shot's start (last shot + 2s). ALL captured in ONE run.
    starts = [t for t, _ in shots]
    ats = [
        start + ((starts[i + 1] if i + 1 < len(starts) else start + 4.0) - start) * 0.5
        for i, (start, _) in enumerate(shots)
    ]
    frames = []
    with tempfile.TemporaryDirectory() as td:
        paths = capture_all(ats, os.path.join(td, "f.png"))
        for i, ((_, label), at, p) in enumerate(zip(shots, ats, paths)):
            ok = os.path.exists(p)
            print(f"  [{i:02d}] t={at:6.1f}s  {label}  {'ok' if ok else 'FAILED'}")
            frames.append((p if ok else None, label, at))

        thumb_w = int(os.environ.get("MARTIN_SHEET_THUMB_W", "320"))
        thumb_h = thumb_w * 9 // 16
        cols = int(os.environ.get("MARTIN_SHEET_COLS", "0")) or max(1, round(len(frames) ** 0.5))
        rows = (len(frames) + cols - 1) // cols
        pad, bar = 6, 16
        cell_h = thumb_h + bar
        sheet = Image.new("RGB", (cols * thumb_w + (cols + 1) * pad, rows * cell_h + (rows + 1) * pad), (12, 12, 14))
        draw = ImageDraw.Draw(sheet)
        for i, (p, label, at) in enumerate(frames):
            cx = pad + (i % cols) * (thumb_w + pad)
            cy = pad + (i // cols) * (cell_h + pad)
            if p:
                th = Image.open(p).convert("RGB").resize((thumb_w, thumb_h), Image.LANCZOS)
                sheet.paste(th, (cx, cy))
            draw.text((cx + 3, cy + thumb_h + 2), f"{i:02d} {at:.1f}s {label}"[:54], fill=(200, 200, 210))
        sheet.save(out)
    print(f"wrote {out}  ({cols}x{rows} grid)")


if __name__ == "__main__":
    main()
