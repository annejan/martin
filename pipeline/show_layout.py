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
  * 3D (the REAL .ply clouds): loads each splat from `assets/`, applies the same
    pipeline (normalize → base-flip → *scale → +pos) and scatters them — a small
    panel from the camera angle, plus a big standalone `<out>_3d.png` with the
    camera POV + a bird's-eye 3/4 so the structure (e.g. the ridge wrap) is
    obvious. Overlap detection flags props sitting in each other's footprint.

Geometry mirrors the engine: each part is normalized so its largest dim =
NORMALIZE_EXTENT (2.0) then placed by `@pos *scale` (scale is per-axis, the
`*sx,sy,sz` form); the orbit camera sits at
  cam = target + dist*(cos p*cos y, sin p, cos p*sin y), looking at target
(see src/camera.rs). Heights use a calibratable prop model (see PROP_HALF_H).

Usage:
  python3 pipeline/show_layout.py productions/camping/campsite-max.show --cam 4
  python3 pipeline/show_layout.py <show> --cam 0 --az-off 20   # tune 3D azimuth to match a render
"""
import argparse
import math
import os
import re

import matplotlib
import numpy as np

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.patches import Ellipse

NORMALIZE_EXTENT = 2.0  # src/scene/mod.rs — each part scaled so largest dim = this
# A part spans ~±(NORMALIZE_EXTENT/2)=±1 after normalize, so world half-extent ≈ scale.
# Disc/round content (terrain) reaches ~1.05 (90th-pct radius of a unit disc ≈ 0.95).
DISC_FACTOR = 1.05
# Prop vertical half-height as a fraction of scale.y (tuned so trees sit ON the field
# in renders, not floating — calibrated against campsite-max.show / campsite renders).
PROP_HALF_H = 0.95

# Rough footprint radius (XZ) per shape category, as a fraction of scale — for the
# top-down circles + the side bars. Trees/fire are slim; tent is a box; ridge is wide.
FOOTPRINT = {
    "pine": 0.45, "flame": 0.4, "tent": 0.7, "moon": 1.0, "mountains": 1.0,
    # mesh:.glb props (no .ply to measure → name-based footprint, fraction of scale):
    "paard": 1.0, "bier": 0.4, "bitterbal": 0.55, "doggo": 0.65,
}
COLOR = {
    "pine": "#2e8b32", "flame": "#ff6a1a", "tent": "#9a6cd0", "moon": "#cfcfcf",
    "mountains": "#6f7fa6", "terrain": "#5aa54a",
    "paard": "#8a5a3c", "bier": "#e0a52a", "bitterbal": "#b5651d", "doggo": "#dcdcdc",
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
            pos, scale, travel = [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], None
            for t in toks[i:]:
                if t.startswith("@"):
                    pos = vec3(t[1:])
                elif t.startswith("*"):
                    scale = vec3(t[1:], (1.0, 1.0, 1.0))
                elif t.startswith("travel:"):
                    travel = vec3(t[len("travel:"):].split("@", 1)[0])  # eased RESTING target
            # plot a travel: prop where it COMES TO REST (target), not its off-screen @pos start.
            stage.append({"name": name, "cat": category(name),
                          "pos": travel if travel else pos, "scale": scale})
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


def parse_ply(path, n=2500):
    """Read a martin sh0 binary .ply (14 f32/vertex) → (positions Nx3, colors Nx3 in 0..1), subsampled."""
    with open(path, "rb") as f:
        blob = f.read()
    end = blob.index(b"end_header\n") + len(b"end_header\n")
    head = blob[:end].decode("ascii", "replace")
    count = int(re.search(r"element vertex (\d+)", head).group(1))
    arr = np.frombuffer(blob[end:end + count * 14 * 4], dtype="<f4").reshape(count, 14)
    if count > n:  # even stride subsample (clouds are unordered, so this is representative)
        arr = arr[:: max(1, count // n)]
    pos = arr[:, 0:3].astype(np.float64)
    col = np.clip(arr[:, 11:14] * 0.2820948 + 0.5, 0, 1)  # f_dc (SH0) → rgb
    return pos, col


def load_ply_world(obj, asset_dir, n=2500):
    """Load a splat .ply and apply the engine pipeline: normalize → base-flip(Y,Z) → *scale → +pos."""
    path = os.path.join(asset_dir, obj["name"])
    if not os.path.exists(path) or not path.endswith(".ply"):
        return None, None  # mesh:.glb props have no sh0 .ply to sample → fall back to name footprint
    pos, col = parse_ply(path, n)
    c = pos.mean(axis=0)
    d = np.linalg.norm(pos - c, axis=1)
    s = (NORMALIZE_EXTENT * 0.5) / max(np.percentile(d, 90), 1e-6)  # normalize_to (90th-pct radius)
    pos = (pos - c) * s
    pos[:, 1] *= -1.0  # base rotation = 180° about X → negate Y …
    pos[:, 2] *= -1.0  # … and Z
    pos = pos * np.array(obj["scale"]) + np.array(obj["pos"])
    return pos, col


def plot_scene_3d(ax, props, fg, asset_dir, elev, azim, cam=None):
    """Scatter the real .ply clouds (engine-transformed) in 3D as (X, Z-depth, Y-up), from a given view."""
    fcx, fcz, fx, fz, fy = fg
    th = np.linspace(0, 2 * math.pi, 80)
    ax.plot(fcx + fx * np.cos(th), fcz + fz * np.sin(th), fy, color="#3c7a32", lw=1, alpha=0.7)
    ax.set_facecolor("#0b0e1a")  # night backdrop → the splat colours pop
    missing = []
    for o in props:
        if o["cat"] == "moon":
            continue  # far backdrop (z≈-50) — would blow the axes
        wp, col = load_ply_world(o, asset_dir, 9000 if o["cat"] in ("terrain", "mountains") else 4500)
        if wp is None:
            missing.append(o["name"])
            continue
        ax.scatter(wp[:, 0], wp[:, 2], wp[:, 1], c=col, s=5, alpha=0.8, depthshade=False, linewidths=0)
    if cam is not None:
        cw = np.array(cam_world(cam))
        tgt = np.array(cam["pos"], dtype=float)
        ax.scatter([cw[0]], [cw[2]], [cw[1]], c="red", marker="^", s=110)
        # camera FRUSTUM: a red view-cone from the camera through its FOV to past the camp, so you can
        # SEE what the shot captures. Engine default FOV = π/4 vertical; horizontal from 16:9 aspect.
        fwd = tgt - cw
        fwd /= max(np.linalg.norm(fwd), 1e-6)
        right = np.cross(fwd, [0.0, 1.0, 0.0])
        right /= max(np.linalg.norm(right), 1e-6)
        tup = np.cross(right, fwd)
        vfov = math.pi / 4
        hfov = 2 * math.atan(math.tan(vfov / 2) * 16 / 9)
        L = np.linalg.norm(tgt - cw) + 6.0  # reach past the camp
        hw, hh = L * math.tan(hfov / 2), L * math.tan(vfov / 2)
        ctr = cw + fwd * L
        corners = [ctr + sr * right * hw + su * tup * hh for sr in (-1, 1) for su in (-1, 1)]
        P = lambda v: ([v[0]], [v[2]], [v[1]])  # noqa: E731 — to mpl (x, z-depth, y-up)
        for c in corners:  # camera apex → each far corner
            ax.plot([cw[0], c[0]], [cw[2], c[2]], [cw[1], c[1]], color="red", lw=0.8, alpha=0.6)
        loop = [corners[0], corners[1], corners[3], corners[2], corners[0]]  # far rectangle
        ax.plot([c[0] for c in loop], [c[2] for c in loop], [c[1] for c in loop],
                color="red", lw=0.8, alpha=0.6)
        ax.plot([cw[0], ctr[0]], [cw[2], ctr[2]], [cw[1], ctr[1]], color="orange", lw=1.2)  # look dir
    ax.view_init(elev=elev, azim=azim)
    ax.set_xlabel("X")
    ax.set_ylabel("Z (depth)")
    ax.set_zlabel("Y up")
    ax.set_xlim(-13, 13)
    ax.set_ylim(-17, 7)
    ax.set_zlim(-3, 3.5)
    ax.set_box_aspect((26, 24, 6.5))
    ax.grid(False)
    # martin→matplotlib (X,Z,Y) swaps two axes = a REFLECTION → the view came out mirrored
    # (tent on the wrong side). Inverting X restores a proper rotation so left/right matches the render.
    ax.invert_xaxis()
    return missing


def draw(path, cam_idx, hfov, out, asset_dir="assets", az_off=0.0):
    props, cams = parse_show(path)
    field = field_of(props)
    cam = cams[cam_idx] if cams and cam_idx < len(cams) else None

    # footprint radius (XZ) per prop + pairwise OVERLAP detection (props sitting in each other's area).
    # Use the REAL .ply horizontal extent (90th-pct radius) so wide shapes (the tent!) aren't under-sized.
    fld = [o for o in props if o["cat"] not in ("terrain", "moon", "mountains")]
    for o in fld:
        wp, _ = load_ply_world(o, "assets", 3000)
        if wp is not None:
            rad = np.hypot(wp[:, 0] - o["pos"][0], wp[:, 2] - o["pos"][2])
            o["r"] = float(np.percentile(rad, 90))
        else:
            o["r"] = FOOTPRINT.get(o["cat"], 0.4) * max(o["scale"])
    overlaps = []
    for i in range(len(fld)):
        for j in range(i + 1, len(fld)):
            a, b = fld[i], fld[j]
            d = math.hypot(a["pos"][0] - b["pos"][0], a["pos"][2] - b["pos"][2])
            if d < (a["r"] + b["r"]) * 1.1:  # 1.1 → margin, so visual touching is caught too
                overlaps.append((a, b, d))

    fig = plt.figure(figsize=(21, 7))
    axt = fig.add_subplot(1, 3, 1)
    axs = fig.add_subplot(1, 3, 2)
    ax3 = fig.add_subplot(1, 3, 3, projection="3d")
    fig.suptitle(f"{path}  —  camera #{cam_idx}" + (f" (t={cam['t']})" if cam else ""), fontsize=11)

    # ---- field geometry (world) ----
    fx = field["scale"][0] * DISC_FACTOR if field else 5.0
    fz = field["scale"][2] * DISC_FACTOR if field else 5.0
    fcx, fcz = (field["pos"][0], field["pos"][2]) if field else (0, 0)
    # props rest on the TOP of the field's undulation (≈ pos.y + amplitude), not its mean —
    # calibrated against grounded renders (hero t=14 / god-view t=11). UNDUL = 0.06 in splatgen.
    famp = 0.06 * DISC_FACTOR * field["scale"][1] if field else 0.27
    fy = (field["pos"][1] + famp) if field else -1.5
    # REAL field surface from the terrain .ply (60th-pct top) — used for the side view + grounding so
    # the field line and the prop bars agree (no false SINK). Falls back to the model fy.
    if field is not None:
        _tp, _ = load_ply_world(field, asset_dir, 9000)
        if _tp is not None:
            fy = float(np.percentile(_tp[:, 1], 60))

    # ===== TOP-DOWN (X horizontal, Z vertical; camera looks toward -Z by convention) =====
    axt.set_title("top-down (X→, Z↑)  ·  is it ON the field & IN frame?")
    axt.add_patch(Ellipse((fcx, fcz), 2 * fx, 2 * fz, fill=True, fc="#bfe3b0", ec="#5aa54a", lw=2, zorder=0))
    # safe-zone ring (~0.85 radius): props OUTSIDE it sit near the rim → bases clip at the horizon
    axt.add_patch(Ellipse((fcx, fcz), 2 * fx * 0.85, 2 * fz * 0.85, fill=False, ec="orange", ls="--", lw=1, alpha=0.6, zorder=1))
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
            wp, _ = load_ply_world(o, asset_dir, 3000)  # REAL base/top (the model lies for flat shapes)
            if wp is not None:
                base, top = float(wp[:, 1].min()), float(wp[:, 1].max())
            else:
                base, top = o["pos"][1] - PROP_HALF_H * o["scale"][1], o["pos"][1] + PROP_HALF_H * o["scale"][1]
            float_sink = ""
            if base > fy + 0.15:
                float_sink = " ⚠FLOAT"
            elif base < fy - 0.20:
                float_sink = " ⚠SINK"
            axs.plot([d, d], [base, top], color=COLOR.get(o["cat"], "#888"), lw=6, alpha=0.8)
            axs.plot([0, d], [cw[1], base], "r:", lw=0.8, alpha=0.5)  # sight ray to base
            axs.annotate(o["cat"] + float_sink, (d, top), ha="center", va="bottom", fontsize=7)
        axs.set_xlabel("depth from camera →")
    axs.set_ylabel("world Y (up)")
    axs.grid(True, alpha=0.3)

    # ===== 3D (real .ply clouds, engine transform) — small panel from the camera angle =====
    fg = (fcx, fcz, fx, fz, fy)
    cam_az = math.degrees(cam["yaw"]) + az_off if cam else -60
    cam_el = math.degrees(cam["pitch"]) if cam else 25
    ax3.set_title("3D — real splats, camera angle")
    missing = plot_scene_3d(ax3, props, fg, asset_dir, cam_el, cam_az, cam)
    if missing:
        print(f"  (3D: missing .ply — run a build to generate: {', '.join(missing)})")

    fig.tight_layout()
    fig.savefig(out, dpi=130)
    print(f"wrote {out}")

    # ===== standalone BIG 3D — two angles (camera POV + a clear bird's-eye 3/4) =====
    out3d = out[:-4] + "_3d.png" if out.endswith(".png") else out + "_3d.png"
    f2 = plt.figure(figsize=(24, 12))
    a = f2.add_subplot(1, 2, 1, projection="3d")
    a.set_title(f"camera POV (cam #{cam_idx})")
    plot_scene_3d(a, props, fg, asset_dir, cam_el, cam_az, cam)
    b = f2.add_subplot(1, 2, 2, projection="3d")
    b.set_title("bird's-eye 3/4 (structure)")
    plot_scene_3d(b, props, fg, asset_dir, 45, -60, cam)
    f2.tight_layout()
    f2.savefig(out3d, dpi=150)
    print(f"wrote {out3d}")
    # text summary
    print(f"field: centre=({fcx},{fcz}) radius X={fx:.2f} Z={fz:.2f} surface Y={fy:.2f}")
    for o in props:
        if o["cat"] in ("terrain", "moon", "mountains"):
            continue
        x, z = o["pos"][0], o["pos"][2]
        inside = ((x - fcx) / fx) ** 2 + ((z - fcz) / fz) ** 2 <= 1.0
        base = o["pos"][1] - PROP_HALF_H * o["scale"][1]
        ratio = ((x - fcx) / fx) ** 2 + ((z - fcz) / fz) ** 2  # 1.0 = on the ellipse edge
        flag = "" if inside else " OFF-FIELD"
        if inside and ratio > 0.72:  # near the rim → base clips at the horizon ("achter onder")
            flag += " NEAR-RIM"
        # FLOAT/SINK comes from the REAL .ply grounding report below (the model half-height is only a
        # rough footprint and lies for flat shapes like the tent) — here just position flags.
        print(f"  {o['cat']:9} @({x:+.2f},{z:+.2f}) rim {ratio:.2f}{flag}")
    for a, b, d in overlaps:
        print(f"  ⚠ OVERLAP: {a['cat']} & {b['cat']} (gap {d:.2f} < radii {a['r'] + b['r']:.2f})")
    # FRUSTUM membership: is each prop actually IN the shot for this camera? (catches the
    # "disappearing tent" — a prop that's in 3D but outside the camera's FOV cone.)
    if cam is not None:
        cw = np.array(cam_world(cam))
        fwd = np.array(cam["pos"], float) - cw
        fwd /= max(np.linalg.norm(fwd), 1e-6)
        right = np.cross(fwd, [0.0, 1.0, 0.0])
        right /= max(np.linalg.norm(right), 1e-6)
        tup = np.cross(right, fwd)
        vfov = math.pi / 4
        hfov = 2 * math.atan(math.tan(vfov / 2) * 16 / 9)
        # screen position normalized to -1..1 (0 = dead centre = boring; ±0.33 = rule-of-thirds line;
        # ±1 = frame edge). Compose subjects toward ±0.33, NOT 0. Terrain skipped (it's the floor).
        print(f"composition (cam #{cam_idx}, FOV {math.degrees(hfov):.0f}°×{math.degrees(vfov):.0f}°)  [screen x,y in -1..1; ±0.33=thirds]:")
        hh, vh = math.degrees(hfov / 2), math.degrees(vfov / 2)
        for o in props:
            if o["cat"] == "terrain":
                continue
            v = np.array(o["pos"], float) - cw
            depth = float(np.dot(v, fwd))
            if depth <= 0.05:
                print(f"  {o['cat']:9} BEHIND camera")
                continue
            sx = math.degrees(math.atan2(float(np.dot(v, right)), depth)) / hh
            sy = math.degrees(math.atan2(float(np.dot(v, tup)), depth)) / vh
            if abs(sx) > 1 or abs(sy) > 1:
                note = "OUT of frame"
            elif abs(sx) < 0.14 and abs(sy) < 0.14:
                note = "⚠ DEAD-CENTRE (boring)"
            elif (0.22 <= abs(sx) <= 0.45) or (0.22 <= abs(sy) <= 0.45):
                note = "✓ near a third"
            else:
                note = ""
            print(f"  {o['cat']:9} screen=({sx:+.2f},{sy:+.2f})  {note}")
    # GROUNDING from the REAL .ply geometry: each prop's true base Y vs the field surface, and the
    # @y that would sit it exactly on the field (so it neither floats nor clips through the ground).
    terr = next((o for o in props if o["cat"] == "terrain"), None)
    surf = fy
    if terr is not None:
        tp, _ = load_ply_world(terr, asset_dir, 9000)
        if tp is not None:
            surf = float(np.percentile(tp[:, 1], 60))  # the field's typical top surface
    print(f"grounding (real .ply, field surface Y≈{surf:+.2f}):")
    for o in props:
        if o["cat"] in ("terrain", "moon", "mountains"):
            continue
        wp, _ = load_ply_world(o, asset_dir, 5000)
        if wp is None:
            continue
        rb = float(wp[:, 1].min())
        gap = rb - surf
        tag = "FLOAT" if gap > 0.06 else ("CLIP/sunk" if gap < -0.06 else "ok")
        print(f"  {o['cat']:9} real base Y={rb:+.2f} (gap {gap:+.2f} {tag})  → @y {o['pos'][1]:+.2f}→{o['pos'][1] - gap:+.2f}")


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("show")
    ap.add_argument("--cam", type=int, default=0, help="camera waypoint index")
    ap.add_argument("--hfov", type=float, default=73.0,
                    help="horizontal FOV (deg); engine default π/4 vertical ≈ 73° horiz @16:9")
    ap.add_argument("-o", "--out", default="/tmp/layout.png")
    ap.add_argument("--assets", default="assets", help="dir holding the generated .ply clouds")
    ap.add_argument("--az-off", type=float, default=0.0, help="azimuth offset (deg) to match the render")
    a = ap.parse_args()
    draw(a.show, a.cam, a.hfov, a.out, a.assets, a.az_off)
