# SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
# SPDX-License-Identifier: MIT
"""
ply_subsample.py — thin out a martin sh0 binary PLY (14 float32 props/point) to N points.

Why: the max-density city captures (43 cm splats, 7-8M points, ~420 MB each) are the AUTHORING
assets. A handout build — `--features bundle`, one self-contained executable on a USB stick — wants
a fraction of that: the executable stays small and the flight stays readable on unknown hardware.

Sampling is a deterministic uniform stride over the file's point order (which is tile/scan order,
so it keeps spatial coverage) — no RNG, so a rebuild reproduces the same cloud bit-for-bit, which
matters for `pair=match` seam morphs between segments.

Usage:  pipeline/ply_subsample.py IN.ply OUT.ply N
"""
import sys, numpy as np

src, dst, n_target = sys.argv[1], sys.argv[2], int(sys.argv[3])

with open(src, "rb") as f:
    hdr = b""
    while b"end_header\n" not in hdr:
        hdr += f.read(1)
    txt = hdr.decode("utf-8", "replace")
    n = int([l for l in txt.splitlines() if l.startswith("element vertex")][0].split()[-1])
    props = [l for l in txt.splitlines() if l.startswith("property ")]
    assert all(p.startswith("property float ") for p in props), "expected all-float32 props"
    stride = len(props)
    data = np.fromfile(f, dtype="<f4", count=n * stride).reshape(n, stride)

if n_target >= n:
    idx = np.arange(n)
else:
    # deterministic uniform stride-with-jitter: keeps spatial coverage, no RNG seed worries
    idx = np.linspace(0, n - 1, n_target).astype(np.int64)

out = data[idx]
new_hdr = txt.replace(f"element vertex {n}", f"element vertex {len(idx)}")
with open(dst, "wb") as f:
    f.write(new_hdr.encode("utf-8"))
    out.astype("<f4").tofile(f)
print(f"{src}: {n} -> {len(idx)} points ({stride} props)")
