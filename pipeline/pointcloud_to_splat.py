#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
# SPDX-License-Identifier: MIT
"""Convert a plain colored point cloud .ply (e.g. an Open3D / Luma export: x y z + r g b) into a
martin-format **Gaussian-splat** .ply (the sh0 layout splatgen writes), so a captured scene that isn't
already 3DGS can be loaded + morphed like any other splat cloud.

Each point becomes one small ISOTROPIC gaussian: its radius is set to the local inter-point spacing
(so the disks just cover the surface, no gaps / no haze), color → degree-0 SH DC, identity rotation.
Subsamples to --count (the iGPU OOMs ~2.5M; ~400k is plenty for a backdrop).

    pipeline/pointcloud_to_splat.py "assets/beach/colored-pointcloud 5.ply" assets/hamtin.ply --count 400000

martin .ply layout (56 B/vertex, little-endian float32): x y z | scale_0..2 (ln radius) | opacity (logit)
| rot_0..3 (quat wxyz, identity 1,0,0,0) | f_dc_0..2 ((c-0.5)/0.2820948).
"""
import argparse
import struct
import sys


def parse_header(data):
    """Return (vertex_count, stride_bytes, field_offsets, header_len). Supports double/float/uchar."""
    end = data.find(b"end_header\n") + len(b"end_header\n")
    lines = data[:end].decode("ascii", "replace").splitlines()
    if not lines or lines[0].strip() != "ply":
        sys.exit("not a .ply")
    if "format binary_little_endian" not in "\n".join(lines):
        sys.exit("only binary_little_endian .ply supported")
    count = 0
    sizes = {"double": 8, "float": 4, "float32": 4, "uchar": 1, "uint8": 1, "char": 1, "int": 4}
    fmt = {"double": "d", "float": "f", "float32": "f", "uchar": "B", "uint8": "B", "char": "b", "int": "i"}
    offset = 0
    fields = {}  # name -> (offset, struct_char)
    for ln in lines:
        p = ln.split()
        if not p:
            continue
        if p[0] == "element" and p[1] == "vertex":
            count = int(p[2])
        elif p[0] == "property":
            ty, name = p[1], p[2]
            if ty not in sizes:
                sys.exit(f"unsupported property type {ty}")
            fields[name] = (offset, fmt[ty])
            offset += sizes[ty]
    return count, offset, fields, end


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("src")
    ap.add_argument("dst")
    ap.add_argument("--count", type=int, default=400_000, help="target splat count (subsample)")
    ap.add_argument("--opacity", type=float, default=0.9)
    ap.add_argument("--overlap", type=float, default=1.4, help="radius = robust spacing * overlap")
    ap.add_argument("--radius", type=float, default=0.0, help="fixed splat radius in .ply units (overrides spacing)")
    args = ap.parse_args()

    data = open(args.src, "rb").read()
    n, stride, fields, hlen = parse_header(data)
    for need in ("x", "y", "z"):
        if need not in fields:
            sys.exit(f"point cloud missing '{need}'")
    has_rgb = all(c in fields for c in ("red", "green", "blue"))
    body = memoryview(data)[hlen:]

    step = max(1, n // args.count)
    kept = list(range(0, n, step))

    # First pass: positions (for the bbox → spacing → radius) + colors.
    def read(i, name):
        off, ch = fields[name]
        return struct.unpack_from("<" + ch, body, i * stride + off)[0]

    xs = [read(i, "x") for i in kept]
    ys = [read(i, "y") for i in kept]
    zs = [read(i, "z") for i in kept]

    # ROBUST extent: the 1st..99th percentile of each axis, not min/max — a handful of stray outlier
    # points (common in phone captures) otherwise blow up the bbox and make the spacing estimate (hence
    # the radius) wildly wrong + count-dependent. --radius overrides the estimate entirely.
    def pct(v, p):
        s = sorted(v)
        return s[min(len(s) - 1, max(0, int(p * len(s))))]

    ext = [pct(a, 0.99) - pct(a, 0.01) for a in (xs, ys, zs)]
    vol = max(ext[0] * ext[1] * ext[2], 1e-9)
    spacing = (vol / max(len(kept), 1)) ** (1.0 / 3.0)
    radius = args.radius if args.radius > 0 else spacing * args.overlap
    ln_r = struct.pack("<f", __import__("math").log(max(radius, 1e-6)))
    op_logit = struct.pack("<f", __import__("math").log(args.opacity / (1.0 - args.opacity)))
    rot = struct.pack("<ffff", 1.0, 0.0, 0.0, 0.0)

    hdr = (
        "ply\nformat binary_little_endian 1.0\ncomment martin: converted from a colored point cloud\n"
        f"element vertex {len(kept)}\n"
        "property float x\nproperty float y\nproperty float z\n"
        "property float scale_0\nproperty float scale_1\nproperty float scale_2\n"
        "property float opacity\n"
        "property float rot_0\nproperty float rot_1\nproperty float rot_2\nproperty float rot_3\n"
        "property float f_dc_0\nproperty float f_dc_1\nproperty float f_dc_2\nend_header\n"
    ).encode("ascii")

    out = bytearray(hdr)
    out += bytearray(len(kept) * 56)
    mv = memoryview(out)[len(hdr):]
    o = 0
    for k, i in enumerate(kept):
        struct.pack_into("<fff", mv, o, xs[k], ys[k], zs[k]); o += 12
        mv[o:o + 12] = ln_r * 3; o += 12  # scale_0..2
        mv[o:o + 4] = op_logit; o += 4
        mv[o:o + 16] = rot; o += 16
        if has_rgb:
            r = read(i, "red") / 255.0
            g = read(i, "green") / 255.0
            b = read(i, "blue") / 255.0
        else:
            r = g = b = 0.5
        struct.pack_into("<fff", mv, o, (r - 0.5) / 0.2820948, (g - 0.5) / 0.2820948, (b - 0.5) / 0.2820948)
        o += 12

    open(args.dst, "wb").write(out)
    print(f"wrote {args.dst}: {len(kept)} splats (from {n} points, step {step}), radius {radius:.4f}")


if __name__ == "__main__":
    main()
