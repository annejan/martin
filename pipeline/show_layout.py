# SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
# SPDX-License-Identifier: MIT
"""
show_layout.py — a fast, GPU-free LAYOUT PREVIEW for a martin `.show` [stage].

The pain it removes: placing props in blind 3D world coords and only catching
off-field / out-of-frame / occluded / floating mistakes after a full GPU render.
This parses the [stage] + [camera] and plots, in <1s:

  * TOP-DOWN map (X-Z, exact): the field ellipse, every prop (footprint + label),
    the camera position, its look direction + FOV wedge. Answers "is it ON the
    field? IN frame? well spaced? clear of the fire?".
  * SIDE elevation (depth-vs-height, exact): the field surface, each prop's
    base→top bar, the camera + sight-rays → flags FLOAT (base above ground),
    SINK (base below ground), and rim OCCLUSION.

Geometry mirrors the engine: each part is normalized so its largest dim =
NORMALIZE_EXTENT (2.0) then placed by `@pos *scale` (scale is per-axis, the
`*sx,sy,sz` form); the orbit camera sits at
  cam = target + dist*(cos p*cos y, sin p, cos p*sin y), looking at target
(see src/camera.rs). Heights use a calibratable prop model (see PROP_HALF_H).

Usage:
  python3 pipeline/show_layout.py productions/camping/hero.show --cam 4 -o /tmp/layout.png
  python3 pipeline/show_layout.py productions/camping/hero.show --cam 4 --hfov 50
"""
import argparse
import math
import re
import sys

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.patches import Ellipse, Polygon

NORMALIZE_EXTENT = 2.0  # src/scene/mod.rs — each part scaled so largest dim = this
# A part spans ~±(NORMALIZE_EXTENT/2)=±1 after normalize, so world half-extent ≈ scale.
# Disc/round content (terrain) reaches ~1.05 (90th-pct radius of a unit disc ≈ 0.95).
DISC_FACTOR = 1.05
# Prop vertical half-height as a fraction of scale.y (tuned so trees sit ON the field
# in renders, not floating — calibrated against hero.show t=14 / campsite renders).
PROP_HALF_H = 0.95

# Rough footprint radius (XZ) per shape category, as a fraction of scale — for the
# top-down circles + the side bars. Trees/fire are slim; tent is a box; ridge is wide.
FOOTPRINT = {"pine": 0.45, "flame": 0.4, "tent": 0.7, "moon": 1.0, "mountains": 1.0}
COLOR = {
    "pine": "#2e8b32", "flame": "#ff6a1a", "tent": "#9a6cd0", "moon": "#cfcfcf",
    "mountains": "#6f7fa6", "terrain": "#5aa54a",
}


def vec3(s, default=(0.0, 0.0, 0.0)):
    parts = [p for p in s.split(",") if p != ""]
    try:
        v = [float(p) for p in parts]
    except ValueError:
        return list(default)
    if len(v) == 1:
        return [v[0], v[0], v[0]]
    while len(v) < 3:
        v.append(0.0)
    return v[:3]


def category(name):
    stem = name.split("/")[-1].replace(".ply", "").replace(".glb", "")
    for key in COLOR:
        if key in stem:
            return key
    return stem


def parse_show(path):
    """Return (props, cameras). props: list of dicts {name,cat,pos,scale}. cameras: list of dicts."""
    stage, cams, section = [], [], None
    for raw in open(path):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        m = re.match(r"\[(\w+)\]", line)
        if m:
            section = m.group(1)
            continue
        if section == "stage":
            toks = line.split()
            head = []
            i = 0
            while i < len(toks) and not (
                toks[i].startswith("@") or toks[i].startswith("*")
                or toks[i] in ("rot", "spin", "sway", "bob", "drift", "in", "out")
            ):
                head.append(toks[i])
                i += 1
            name = " ".join(head)
            name = re.sub(r"^\w+:", "", name)  # strip splat:/mesh:/glb:/...
            pos, scale = [0.0, 0.0, 0.0], [1.0, 1.0, 1.0]
            for t in toks[i:]:
                if t.startswith("@"):
                    pos = vec3(t[1:])
                elif t.startswith("*"):
                    scale = vec3(t[1:], (1.0, 1.0, 1.0))
            stage.append({"name": name, "cat": category(name), "pos": pos, "scale": scale})
        elif section == "camera":
            d = dict(re.findall(r"(\w+)=([-\d.,]+|@@\w+)", line))
            if "yaw" in d:
                cams.append({
                    "t": d.get("t", "?"),
                    "pos": vec3(d.get("pos", "0,0,0")),
                    "dist": float(d.get("dist", 5)),
                    "yaw": float(d.get("yaw", 0)),
                    "pitch": float(d.get("pitch", 0)),
                })
    return stage, cams


def cam_world(cam):
    """Engine orbit math (src/camera.rs): camera world position from target/dist/yaw/pitch."""
    p, y, d = cam["pitch"], cam["yaw"], cam["dist"]
    t = cam["pos"]
    cp, sp, cy, sy = math.cos(p), math.sin(p), math.cos(y), math.sin(y)
    return [t[0] + d * cp * cy, t[1] + d * sp, t[2] + d * cp * sy]


def field_of(props):
    for o in props:
        if o["cat"] == "terrain":
            return o
    return None


def draw(path, cam_idx, hfov, out):
    props, cams = parse_show(path)
    field = field_of(props)
    cam = cams[cam_idx] if cams and cam_idx < len(cams) else None

    # footprint radius (XZ) per prop + pairwise OVERLAP detection (props sitting in each other's area)
    fld = [o for o in props if o["cat"] not in ("terrain", "moon", "mountains")]
    for o in fld:
        o["r"] = FOOTPRINT.get(o["cat"], 0.4) * max(o["scale"])
    overlaps = []
    for i in range(len(fld)):
        for j in range(i + 1, len(fld)):
            a, b = fld[i], fld[j]
            d = math.hypot(a["pos"][0] - b["pos"][0], a["pos"][2] - b["pos"][2])
            if d < (a["r"] + b["r"]) * 0.9:  # 0.9 → allow a slight touch
                overlaps.append((a, b, d))

    fig, (axt, axs) = plt.subplots(1, 2, figsize=(15, 7))
    fig.suptitle(f"{path}  —  camera #{cam_idx}" + (f" (t={cam['t']})" if cam else ""), fontsize=11)

    # ---- field geometry (world) ----
    fx = field["scale"][0] * DISC_FACTOR if field else 5.0
    fz = field["scale"][2] * DISC_FACTOR if field else 5.0
    fcx, fcz = (field["pos"][0], field["pos"][2]) if field else (0, 0)
    # props rest on the TOP of the field's undulation (≈ pos.y + amplitude), not its mean —
    # calibrated against grounded renders (hero t=14 / god-view t=11). UNDUL = 0.06 in splatgen.
    famp = 0.06 * DISC_FACTOR * field["scale"][1] if field else 0.27
    fy = (field["pos"][1] + famp) if field else -1.5

    # ===== TOP-DOWN (X horizontal, Z vertical; camera looks toward -Z by convention) =====
    axt.set_title("top-down (X→, Z↑)  ·  is it ON the field & IN frame?")
    axt.add_patch(Ellipse((fcx, fcz), 2 * fx, 2 * fz, fill=True, fc="#bfe3b0", ec="#5aa54a", lw=2, zorder=0))
    for o in props:
        if o["cat"] in ("terrain", "moon", "mountains"):
            continue
        x, z = o["pos"][0], o["pos"][2]
        r = FOOTPRINT.get(o["cat"], 0.4) * max(o["scale"])
        inside = ((x - fcx) / fx) ** 2 + ((z - fcz) / fz) ** 2 <= 1.0
        axt.add_patch(plt.Circle((x, z), r, fc=COLOR.get(o["cat"], "#888"),
                                 ec="black" if inside else "red", lw=2, alpha=0.85, zorder=3))
        axt.annotate(o["cat"] + ("" if inside else " ⚠OFF-FIELD"), (x, z),
                     ha="center", va="center", fontsize=7, zorder=4)
    for a, b, _ in overlaps:  # red bar between props sitting in each other's area
        axt.plot([a["pos"][0], b["pos"][0]], [a["pos"][2], b["pos"][2]], "r-", lw=3, alpha=0.8, zorder=6)
    if cam:
        cw = cam_world(cam)
        axt.plot(cw[0], cw[2], "k^", ms=12, zorder=5)
        axt.annotate("CAM", (cw[0], cw[2]), textcoords="offset points", xytext=(8, 8), fontsize=8)
        # look direction + FOV wedge toward the target
        look = math.atan2(cam["pos"][2] - cw[2], cam["pos"][0] - cw[0])
        reach = cam["dist"] * 2.2
        half = math.radians(hfov / 2)
        for s in (-1, 1):
            ang = look + s * half
            axt.plot([cw[0], cw[0] + reach * math.cos(ang)], [cw[2], cw[2] + reach * math.sin(ang)],
                     "b--", lw=1, alpha=0.6, zorder=2)
        axt.plot([cw[0], cw[0] + reach * math.cos(look)], [cw[2], cw[2] + reach * math.sin(look)],
                 "b-", lw=1.5, alpha=0.5, zorder=2)
    axt.set_aspect("equal")
    axt.grid(True, alpha=0.3)
    m = max(fx, fz) * 1.4
    axt.set_xlim(fcx - m, fcx + m)
    axt.set_ylim(fcz - m, fcz + m)
    axt.invert_yaxis()  # -Z (far) at top, matching the camera's forward

    # ===== SIDE (depth toward camera = horizontal, world Y = vertical) =====
    # Project along the camera's left/right so depth is "distance from camera along look".
    axs.set_title("side  ·  base on the ground? (FLOAT/SINK)  ·  rim occlusion?")
    axs.axhline(fy, color="#5aa54a", lw=2, label="field surface")
    if cam:
        cw = cam_world(cam)
        # signed depth of a world point along the look direction (0 = camera plane)
        lookv = [cam["pos"][0] - cw[0], cam["pos"][2] - cw[2]]
        ln = math.hypot(*lookv) or 1
        lookv = [lookv[0] / ln, lookv[1] / ln]

        def depth(x, z):
            return (x - cw[0]) * lookv[0] + (z - cw[2]) * lookv[1]

        axs.plot(0, cw[1], "k^", ms=12)
        axs.annotate("CAM", (0, cw[1]), textcoords="offset points", xytext=(6, 6), fontsize=8)
        # field surface extent in depth
        fd0, fd1 = depth(fcx - fx, fcz), depth(fcx + fx, fcz)
        axs.plot([min(fd0, fd1), max(fd0, fd1)], [fy, fy], color="#5aa54a", lw=2)
        for o in props:
            if o["cat"] in ("terrain", "moon", "mountains"):
                continue
            d = depth(o["pos"][0], o["pos"][2])
            half_h = PROP_HALF_H * o["scale"][1]
            base, top = o["pos"][1] - half_h, o["pos"][1] + half_h
            float_sink = ""
            if base > fy + 0.15:
                float_sink = " ⚠FLOAT"
            elif base < fy - 0.40:
                float_sink = " ⚠SINK"
            axs.plot([d, d], [base, top], color=COLOR.get(o["cat"], "#888"), lw=6, alpha=0.8)
            axs.plot([0, d], [cw[1], base], "r:", lw=0.8, alpha=0.5)  # sight ray to base
            axs.annotate(o["cat"] + float_sink, (d, top), ha="center", va="bottom", fontsize=7)
        axs.set_xlabel("depth from camera →")
    axs.set_ylabel("world Y (up)")
    axs.grid(True, alpha=0.3)

    fig.tight_layout()
    fig.savefig(out, dpi=90)
    print(f"wrote {out}")
    # text summary
    print(f"field: centre=({fcx},{fcz}) radius X={fx:.2f} Z={fz:.2f} surface Y={fy:.2f}")
    for o in props:
        if o["cat"] in ("terrain", "moon", "mountains"):
            continue
        x, z = o["pos"][0], o["pos"][2]
        inside = ((x - fcx) / fx) ** 2 + ((z - fcz) / fz) ** 2 <= 1.0
        base = o["pos"][1] - PROP_HALF_H * o["scale"][1]
        flag = "" if inside else " OFF-FIELD"
        flag += " FLOAT" if base > fy + 0.15 else (" SINK" if base < fy - 0.40 else "")
        print(f"  {o['cat']:9} @({x:+.2f},{o['pos'][1]:+.2f},{z:+.2f}) base Y={base:+.2f}{flag}")
    for a, b, d in overlaps:
        print(f"  ⚠ OVERLAP: {a['cat']} & {b['cat']} (gap {d:.2f} < radii {a['r'] + b['r']:.2f})")


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("show")
    ap.add_argument("--cam", type=int, default=0, help="camera waypoint index")
    ap.add_argument("--hfov", type=float, default=73.0,
                    help="horizontal FOV (deg); engine default π/4 vertical ≈ 73° horiz @16:9")
    ap.add_argument("-o", "--out", default="/tmp/layout.png")
    a = ap.parse_args()
    draw(a.show, a.cam, a.hfov, a.out)
