#!/usr/bin/env python3
"""Headless render smoke-test for every martin show — catch crashes / dead-black / missing assets
without sitting through a full render. Complements pipeline/sheet.py (which makes a pretty contact
sheet); this one is the CI gate: it renders a few frames per show and FAILS (exit 1) on a crash or a
show that is black at every probed moment.

How it works (pure tooling over the headless MARTIN_SHOT capture — no engine changes):
  1. MARTIN_VALIDATE=1 dumps the resolved cue timeline (parse-checks the show, gives shot starts),
  2. render 3 frames per show at early / mid / late show-times (MARTIN_SHOT, capped at 45 s wall —
     MARTIN_SHOT_AT is wall-clock, so deep tails of long shows are left to the full record.sh render),
  3. stat each frame (mean luma / max channel / lit-fraction) and classify ok / near-black / BLACK.

A single black frame is a WARNING (many shows open on a dramatic fade-from-black); a show that is
black at EVERY probe, or crashes / fails to parse, is a hard FAILURE.

Usage:
    pipeline/smoke-shows.py                 # all assets/examples + productions shows
    pipeline/smoke-shows.py killshot cities # only shows whose path contains one of these
Honours MARTIN_* env (renders are MARTIN_MUTE=1, MARTIN_FFT=0 for speed). Deterministic.
"""
import glob
import os
import re
import subprocess
import sys

from PIL import Image, ImageStat

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(HERE, "target", "release", "martin")
OUT = os.environ.get("MARTIN_SMOKE_OUT", "/tmp/martin-smoke")
SHOT_RE = re.compile(r"\[(\d+)\]\s+t=\s*([\d.]+)s\s+(.*)")
MAXAT = 45.0  # MARTIN_SHOT_AT is wall-clock; don't wait longer than this per frame.
TIMEOUT = int(MAXAT) + 90


def base_env(extra=None):
    e = dict(os.environ)
    e["BEVY_ASSET_ROOT"] = HERE
    e.setdefault("DISPLAY", ":0")
    e["MARTIN_MUTE"] = "1"
    e.setdefault("MARTIN_FFT", "0")  # skip the spectrum bake — thumbnails don't need it
    if extra:
        e.update(extra)
    return e


def validate(show):
    e = base_env({"MARTIN_SHOW": show, "MARTIN_VALIDATE": "1"})
    try:
        r = subprocess.run([BIN], env=e, capture_output=True, text=True, timeout=200)
    except subprocess.TimeoutExpired:
        return None, ["TIMEOUT in validate"], -1
    starts = [float(m.group(2)) for m in (SHOT_RE.search(l) for l in r.stdout.splitlines()) if m]
    warns = [l.strip() for l in r.stderr.splitlines()
             if re.search(r"error|panic|missing|fail|not 16:9", l, re.I)][:6]
    return starts, warns, r.returncode


def shot(show, at, path):
    e = base_env({"MARTIN_SHOW": show, "MARTIN_SHOT": path, "MARTIN_SHOT_AT": f"{at:.3f}"})
    try:
        r = subprocess.run([BIN], env=e, capture_output=True, text=True, timeout=TIMEOUT)
    except subprocess.TimeoutExpired:
        return "TIMEOUT", None
    if not os.path.exists(path):
        return f"NOFRAME rc={r.returncode}", None
    im = Image.open(path).convert("RGB")
    st = ImageStat.Stat(im)
    g = im.convert("L")
    lit = sum(g.histogram()[17:]) / (g.width * g.height)
    return None, (round(sum(st.mean) / 3, 1), max(e[1] for e in st.extrema), round(lit, 3))


def classify(stat):
    mean, mx, lit = stat
    if mx < 8:
        return "BLACK"
    if lit < 0.005:
        return "near-black"
    return "ok"


def main():
    if not os.path.exists(BIN):
        sys.exit(f"build first: cargo build --release  (missing {BIN})")
    os.makedirs(OUT, exist_ok=True)
    shows = sorted(glob.glob(f"{HERE}/assets/examples/*.show")) + sorted(glob.glob(f"{HERE}/productions/*/*.show"))
    if len(sys.argv) > 1:
        shows = [s for s in shows if any(a in s for a in sys.argv[1:])]

    failures = []
    for show in shows:
        name = os.path.relpath(show, HERE)
        starts, warns, rc = validate(show)
        if starts is None or rc != 0:
            print(f"\n### {name}\n   VALIDATE FAILED rc={rc} {warns}")
            failures.append(f"{name}: validate rc={rc}")
            continue
        last = max(starts) if starts else 6.0
        probe = min(last, MAXAT)
        times = sorted(set([round((min(starts) if starts else 2.0) + 1.0, 1),
                            round(probe * 0.5 + 1.0, 1), round(probe + 0.5, 1)]))
        print(f"\n### {name}  ({len(starts)} shots, last@{last:.1f}s)")
        for w in warns:
            print(f"   ! {w[:120]}")
        verdicts = []
        for i, at in enumerate(times):
            p = f"{OUT}/{name.replace('/', '_')}_{i}.png"
            err, stat = shot(show, at, p)
            if err:
                print(f"   t={at:6.1f}  {err}")
                verdicts.append(err)
            else:
                v = classify(stat)
                print(f"   t={at:6.1f}  mean={stat[0]:5} max={stat[1]:3} lit={stat[2]:.3f}  {v}")
                verdicts.append(v)
        # hard failure only if NOTHING rendered ok anywhere (a truly dead/black show or all-crash)
        if not any(v == "ok" for v in verdicts):
            failures.append(f"{name}: no good frame ({verdicts})")

    print("\n" + ("=" * 60))
    if failures:
        print("SMOKE FAILURES:")
        for f in failures:
            print(f"  ✗ {f}")
        sys.exit(1)
    print(f"smoke ok — {len(shows)} shows render, none dead/black/crashing")


if __name__ == "__main__":
    main()
