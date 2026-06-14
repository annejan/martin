#!/usr/bin/env python3
"""
crop-splat.py — trim floaters from a Gaussian-splat .ply (binary little-endian).

Photogrammetry splats (COLMAP→Brush) come wrapped in sky/ground floaters: sparse
splats far from the subject that inflate the bounding box, wreck auto-framing
(the subject shrinks to a distant dot), and look like noise. This drops them with
two robust filters, preserving every per-splat field (works for sh0 14-float and
sh3 59-float layouts alike).

  1. Radial: keep splats within the Nth distance-percentile of the MEDIAN centre
     (median, not mean → floater-robust).
  2. Opacity: drop near-transparent splats (raw logit < threshold) — most haze.

Usage:
  python3 pipeline/crop-splat.py in.ply out.ply
Env tunables:
  KEEP_PCT=90     keep the closest this-% of splats by distance to median centre
  OPACITY_MIN=-4  drop splats with raw opacity logit below this (sigmoid≈0.018)
  AXIS_CLIP=0     also clip per-axis to [p,100-p] before radial (0=off; e.g. 1)
"""
import sys, os, numpy as np

def read_ply(path):
    with open(path, "rb") as f:
        hdr = b""
        while b"end_header\n" not in hdr:
            c = f.read(1)
            if not c:
                raise SystemExit("no end_header — not a ply?")
            hdr += c
        txt = hdr.decode("latin1")
        if "binary_little_endian" not in txt:
            raise SystemExit("only binary_little_endian plys supported")
        props = [l.split()[2] for l in txt.splitlines() if l.startswith("property float")]
        n = int(next(l for l in txt.splitlines() if l.startswith("element vertex")).split()[-1])
        data = np.frombuffer(f.read(n * len(props) * 4), dtype="<f4").reshape(n, len(props))
    return hdr, txt, props, data

def main():
    if len(sys.argv) != 3:
        raise SystemExit(__doc__)
    src, dst = sys.argv[1], sys.argv[2]
    keep_pct = float(os.environ.get("KEEP_PCT", 90))
    op_min = float(os.environ.get("OPACITY_MIN", -4))
    axis_clip = float(os.environ.get("AXIS_CLIP", 0))

    hdr, txt, props, data = read_ply(src)
    n = len(data)
    ix, iy, iz = props.index("x"), props.index("y"), props.index("z")
    xyz = data[:, [ix, iy, iz]]

    # non-finite positions (NaN/Inf floaters Brush sometimes emits) poison median/percentile
    # and would mask out everything — drop them up front.
    finite = np.isfinite(data).all(axis=1)
    n_bad = int((~finite).sum())
    center = np.median(xyz[finite], axis=0)

    mask = finite.copy()
    if n_bad:
        print(f"  dropped {n_bad} non-finite splats")
    if axis_clip > 0:
        for ax in (ix, iy, iz):
            lo, hi = np.percentile(data[:, ax], [axis_clip, 100 - axis_clip])
            mask &= (data[:, ax] >= lo) & (data[:, ax] <= hi)

    dist = np.linalg.norm(xyz - center, axis=1)
    rcut = np.percentile(dist[mask], keep_pct)
    mask &= dist <= rcut

    if "opacity" in props:
        mask &= data[:, props.index("opacity")] >= op_min

    kept = data[mask]
    # report
    ext_before = (xyz.max(0) - xyz.min(0))
    kxyz = kept[:, [ix, iy, iz]]
    ext_after = (kxyz.max(0) - kxyz.min(0))
    print(f"  in : {n} splats, bbox extent {np.linalg.norm(ext_before):.1f}")
    print(f"  out: {len(kept)} splats ({100*len(kept)/n:.1f}%), bbox extent {np.linalg.norm(ext_after):.1f}")
    print(f"  centre(median)={center.round(2)}  radial cut={rcut:.2f}  opacity_min={op_min}")

    out_hdr = txt.replace(f"element vertex {n}", f"element vertex {len(kept)}").encode("latin1")
    with open(dst, "wb") as f:
        f.write(out_hdr)
        f.write(np.ascontiguousarray(kept, dtype="<f4").tobytes())
    print(f"  wrote {dst} ({os.path.getsize(dst)//1_000_000} MB)")

if __name__ == "__main__":
    main()
