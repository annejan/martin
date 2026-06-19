# SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
# SPDX-License-Identifier: MIT
"""
blender_bridge.py — round-trip a martin `.show` scene through **Blender** for visual composition.

Compose/pose a scene in Blender's viewport, then read it straight back into the `.show`. Runs INSIDE
Blender (it needs `bpy`), driven over the `blender-mcp` MCP server (`uvx blender-mcp` + the Blender
add-on, port 9876; `blender` is symlinked to `blender-5.1`). Typical loop, from the martin repo:

    # in Blender (via mcp__*__execute_blender_code):
    exec(open('/home/annejan/Projects/martin/pipeline/blender_bridge.py').read())
    bridge_import('productions/camping/campsite.show')        # spawn the [stage] + set the camera
    # …user drags/rotates/scales props + orbits the camera (camera-lock is ON)…
    bridge_export('productions/camping/campsite.show')        # write @pos/*scale/rot back, keep all tokens
    bridge_shot('/tmp/blender_scene.png')                     # save a viewport shot to deliver

Then render martin to confirm (it now matches 1:1 — see "WHY IT MATCHES"):

    MARTIN_SHOW=productions/camping/campsite.show MARTIN_RECORD=$REC MARTIN_RES=854x480 \
      MARTIN_PREVIEW_FPS=2 MARTIN_BUDGET=500000 BEVY_ASSET_ROOT=. ./target/release/martin
    DISPLAY=:0 xdg-open <frame>.png      # deliver to the user (MCP viewport shots go to the model only)

──────────────────────────────────────────────────────────────────────────────────────────────────
THE MAPS (martin ↔ Blender), all VERIFIED 1:1 against engine renders:

* COORDINATES. martin world is Y-up; Blender is Z-up. Basis `B = [[1,0,0],[0,0,-1],[0,1,0]]` maps a
  martin-world vector to Blender: `blender = (mx, -mz, my)`; inverse `martin = (bx, bz, -by)`. It's a
  PROPER rotation (det +1) — left/right + drag direction are faithful, nothing is mirrored.

* NORMALIZE. martin centers every part on its CENTROID (mean) and scales by `1/p90` (90th-percentile
  distance from the centroid) so the content spans ~2 units (`NORMALIZE_EXTENT=2`, `morph::normalize_to`).
  A `[stage]` `@pos` is a WORLD position (the normalized centroid sits at the world origin).

* SPLAT `.ply` → point cloud. Brush SH-3 ply: parse the header (count `property float` lines = stride),
  take `x,y,z` + `f_dc_0..2`; `rgb = clip(f_dc*0.2820948 + 0.5, 0, 1)`. Normalize (centroid, 1/p90), apply
  the per-axis `*scale` in raw axes, then map `blender = (nx, nz, -ny)` (= base-flip 180°X + B). Emission
  material reading the `color` attr on a dark world = martin's vivid splat look.

* MESH `.glb` → import native (Blender does Y-up→Z-up), join, normalize to ~2 units, apply `*scale`.

* TEXT. A Blender text object's martin-default orientation is `Rx(90°)` (martin builds `text:` flat in
  world XY, +X right, +Y up — see src/text.rs). Positive scale, NO mirror.

* ROTATION. `rot a,b,c` euler degrees. With Blender world-rot `R_obj` and the prop's default Blender
  orientation `R_base` (identity for splat/mesh, `Rx90` for text): `R_extra = R_obj·R_baseᵀ`, the martin
  token is `(Bᵀ·R_extra·B).to_euler('XYZ')` deg. Inverse (import): `R_obj = B·R_m·Bᵀ·R_base`.

* CAMERA. martin orbit = target + dist·(cosp·cosy, sinp, cosp·siny), look-at(target, +Y). Export from a
  Blender cam world `P` + look target `T`: `dist=|P−T|`, `d=(P−T)/dist`, `yaw=atan2(d.z,d.x)`,
  `pitch=asin(d.y)`, `pos=T`. FOV (load-bearing): martin = `π/4` VERTICAL (Bevy default). Set the Blender
  cam `sensor_fit='VERTICAL'`, `angle=π/4`, render 16:9 — else framing won't match.

WHY IT MATCHES (engine fix, commit d229ba7): a fully-timed `[camera]` track is now authoritative for ANY
show (compose-only included) — no `MARTIN_FLY`, no reel-hero trick. (Before, a compose stage auto-framed
and ignored its `[camera]`.) There is NO render distortion (an earlier "stretch" was camera-pose noise).

ROUND-TRIP INTEGRITY: `bridge_import` tags each spawned object with `ob['_show_line']` = its source line
index; `bridge_export` patches ONLY `@pos`/`*scale`/`rot` on those exact lines, so every martin-only token
(`spin`/`sway`/`bob`/`^deform`/`tint:`/`count:`/`alpha:`/`path:`/`travel:`/`in`/`out`/`~entrance`) is kept.
Programmatic content is NOT poseable: `path:`/`travel:` props show their START `@pos`; a `[reel]` morph is
temporal (load one frame as a backdrop via `bridge_import(..., backdrop='seattle_tight.ply')`).
"""
import bpy, numpy as np, re, math, os
from math import radians, degrees, cos, sin
from mathutils import Matrix, Euler, Vector

ROOT = "/home/annejan/Projects/martin"          # repo root (override if relocated)
ASSET_DIRS = ["assets", "austin_run/exports"]   # searched in order for a referenced asset
B = Matrix(((1, 0, 0), (0, 0, -1), (0, 1, 0)))  # martin-world -> Blender basis
NORMALIZE_EXTENT = 2.0

# ── parsing ─────────────────────────────────────────────────────────────────────────────────────
_KW = ("rot", "spin", "sway", "bob", "drift", "in", "out")
_KWP = ("alpha:", "count:", "tint:", "travel:", "path:", "field:", "^", "~")


def _is_kw(t):
    return t in _KW or any(t.startswith(k) for k in _KWP)


def _vec3(s, d=(0.0, 0.0, 0.0)):
    p = [t for t in s.split(",") if t != ""]
    try:
        v = [float(t) for t in p]
    except ValueError:
        return list(d)
    if len(v) == 1:
        v = v * 3
    while len(v) < 3:
        v.append(0.0)
    return v[:3]


def parse_stage(show_path):
    """Return [{line, kind, name, pos, scale, rot}] for each `[stage]` object (line = file line index)."""
    path = show_path if os.path.isabs(show_path) else os.path.join(ROOT, show_path)
    out, sec = [], None
    for li, raw in enumerate(open(path)):
        line = raw.split("#", 1)[0].strip()
        if not line:
            continue
        m = re.match(r"\[(\w+)\]", line)
        if m:
            sec = m.group(1)
            continue
        if sec != "stage":
            continue
        toks = line.split()
        head, i = [], 0
        while i < len(toks) and not (toks[i].startswith("@") or toks[i].startswith("*") or _is_kw(toks[i])):
            head.append(toks[i])
            i += 1
        full = " ".join(head)
        kind = full.split(":", 1)[0] if ":" in full else "splat"
        name = re.sub(r"^\w+:", "", full)
        pos, scale, rot, travel = [0, 0, 0], [1, 1, 1], [0, 0, 0], None
        j = i
        while j < len(toks):
            t = toks[j]
            if t.startswith("@"):
                pos = _vec3(t[1:])
            elif t.startswith("*"):
                scale = _vec3(t[1:], (1, 1, 1))
            elif t == "rot" and j + 1 < len(toks):
                rot = _vec3(toks[j + 1])
                j += 1
            elif t.startswith("travel:"):
                travel = _vec3(t[len("travel:"):].split("@", 1)[0])
            j += 1
        out.append(dict(line=li, kind=kind, name=name, pos=(travel or pos), scale=scale, rot=rot,
                        has_travel=travel is not None))
    return out


def parse_camera(show_path):
    """Return [{anchor/t, pos, dist, yaw, pitch}] from the `[camera]` track."""
    path = show_path if os.path.isabs(show_path) else os.path.join(ROOT, show_path)
    out, sec = [], None
    for raw in open(path):
        line = raw.split("#", 1)[0].strip()
        m = re.match(r"\[(\w+)\]", line)
        if m:
            sec = m.group(1)
            continue
        if sec != "camera" or "=" not in line:
            continue
        d = dict(re.findall(r"(\w+)=([-\d.,]+|@@\w+)", line))
        if "dist" in d:
            out.append(dict(t=d.get("t", "?"), pos=_vec3(d.get("pos", "0,0,0")),
                            dist=float(d["dist"]), yaw=float(d.get("yaw", 1.4)), pitch=float(d.get("pitch", 0.12))))
    return out


def _find_asset(name):
    for sub in ASSET_DIRS:
        p = os.path.join(ROOT, sub, name)
        if os.path.exists(p):
            return p
    return None


# ── geometry helpers ──────────────────────────────────────────────────────────────────────────────
def m2b(p):
    """martin world position -> Blender."""
    return Vector((p[0], -p[2], p[1]))


def b2m(v):
    """Blender world position -> martin."""
    return (round(v.x, 4), round(v.z, 4), round(-v.y, 4))


def _Robj(rot_deg, Rbase=Matrix.Identity(3)):
    """martin `rot` euler (deg) -> Blender object rotation matrix (import direction)."""
    Rm = Euler((radians(rot_deg[0]), radians(rot_deg[1]), radians(rot_deg[2]))).to_matrix()
    return B @ Rm @ B.transposed() @ Rbase


def _rot_to_martin(ob, Rbase=Matrix.Identity(3)):
    """Blender object world-rotation -> martin `rot` euler degrees (export direction)."""
    Rextra = ob.matrix_world.to_3x3().normalized() @ Rbase.inverted()
    e = (B.transposed() @ Rextra @ B).to_euler("XYZ")
    return tuple(round(degrees(a), 1) for a in e)


def _load_ply(path, n=10000):
    with open(path, "rb") as f:
        blob = f.read()
    end = blob.index(b"end_header\n") + len(b"end_header\n")
    head = blob[:end].decode("ascii", "replace")
    cnt = int([l for l in head.splitlines() if l.startswith("element vertex")][0].split()[-1])
    props = [l.split()[-1] for l in head.splitlines() if l.startswith("property float")]
    arr = np.frombuffer(blob[end:end + cnt * len(props) * 4], dtype="<f4").reshape(cnt, len(props))
    if cnt > n:
        arr = arr[:: max(1, cnt // n)]
    ix = {p: i for i, p in enumerate(props)}
    xyz = arr[:, [ix["x"], ix["y"], ix["z"]]].astype(np.float64)
    if "f_dc_0" in ix:
        rgb = np.clip(arr[:, [ix["f_dc_0"], ix["f_dc_1"], ix["f_dc_2"]]] * 0.2820948 + 0.5, 0, 1)
    else:
        rgb = np.full((len(xyz), 3), 0.7)
    return xyz, rgb


def _emit_mat(name="bridge_splat"):
    m = bpy.data.materials.get(name) or bpy.data.materials.new(name)
    m.use_nodes = True
    nt = m.node_tree
    nt.nodes.clear()
    a = nt.nodes.new("ShaderNodeAttribute")
    a.attribute_name = "color"
    e = nt.nodes.new("ShaderNodeEmission")
    o = nt.nodes.new("ShaderNodeOutputMaterial")
    nt.links.new(a.outputs["Color"], e.inputs["Color"])
    nt.links.new(e.outputs["Emission"], o.inputs["Surface"])
    return m


def _emit_text_mat(rgb):
    m = bpy.data.materials.new("bridge_text")
    m.use_nodes = True
    nt = m.node_tree
    nt.nodes.clear()
    e = nt.nodes.new("ShaderNodeEmission")
    e.inputs["Color"].default_value = (*rgb, 1)
    e.inputs["Strength"].default_value = 2.0
    o = nt.nodes.new("ShaderNodeOutputMaterial")
    nt.links.new(e.outputs["Emission"], o.inputs["Surface"])
    return m


def _bbox_norm(ob):
    bb = ob.bound_box
    dims = [max(c[i] for c in bb) - min(c[i] for c in bb) for i in range(3)]
    return NORMALIZE_EXTENT / max(max(dims), 1e-6)


def _spawn_splat(spec, n=10000):
    path = _find_asset(spec["name"])
    if not path:
        print("bridge: MISSING splat", spec["name"])
        return None
    xyz, rgb = _load_ply(path, n)
    c = xyz.mean(0)
    s = NORMALIZE_EXTENT * 0.5 / max(np.percentile(np.linalg.norm(xyz - c, axis=1), 90), 1e-6)
    nrm = (xyz - c) * s * np.array(spec["scale"])           # per-axis martin scale, raw axes
    bl = np.column_stack([nrm[:, 0], nrm[:, 2], -nrm[:, 1]]).astype(np.float32)  # base-flip + map
    pc = bpy.data.pointclouds.new(spec["name"])
    pc.resize(len(bl))
    pc.attributes["position"].data.foreach_set("vector", bl.ravel())
    rad = 0.02 if any(k in spec["name"] for k in ("terrain", "mountain")) else 0.012
    (pc.attributes.get("radius") or pc.attributes.new("radius", "FLOAT", "POINT")).data.foreach_set(
        "value", np.full(len(bl), rad, np.float32))
    col = pc.attributes.new("color", "FLOAT_COLOR", "POINT")
    col.data.foreach_set("color", np.column_stack([rgb, np.ones(len(bl))]).astype(np.float32).ravel())
    pc.materials.append(_emit_mat())
    ob = bpy.data.objects.new("PROP." + spec["name"], pc)
    bpy.context.collection.objects.link(ob)
    ob.matrix_basis = Matrix.Translation(m2b(spec["pos"])) @ _Robj(spec["rot"]).to_4x4()
    return ob


def _spawn_mesh(spec):
    path = _find_asset(spec["name"])
    if not path:
        print("bridge: MISSING mesh", spec["name"])
        return None
    before = set(bpy.data.objects)
    bpy.ops.import_scene.gltf(filepath=path)
    new = [o for o in bpy.data.objects if o not in before]
    meshes = [o for o in new if o.type == "MESH"]
    if not meshes:
        for o in new:
            bpy.data.objects.remove(o, do_unlink=True)
        return None
    for o in bpy.data.objects:
        o.select_set(False)
    for o in meshes:
        o.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    if len(meshes) > 1:
        bpy.ops.object.join()
    ob = bpy.context.active_object
    for o in list(bpy.data.objects):
        if o in new and o != ob and o.type != "MESH":
            bpy.data.objects.remove(o, do_unlink=True)
    us = _bbox_norm(ob) * spec["scale"][0]
    ob.name = "PROP." + spec["name"].replace(".glb", "")
    ob.matrix_basis = (Matrix.Translation(m2b(spec["pos"])) @ _Robj(spec["rot"]).to_4x4()
                       @ Matrix.Diagonal((us, us, us, 1.0)))
    return ob


def _spawn_text(spec):
    bpy.ops.object.text_add()
    t = bpy.context.object
    t.name = "PROP." + spec["name"][:12]
    t.data.body = spec["name"]
    t.data.extrude = 0.04
    t.data.align_x = "CENTER"
    t.data.align_y = "CENTER"
    bpy.context.view_layer.update()
    fit = NORMALIZE_EXTENT / max(t.dimensions.x, 1e-6) * spec["scale"][0]
    Rb = Euler((radians(90), 0, 0)).to_matrix()
    t.matrix_basis = (Matrix.Translation(m2b(spec["pos"])) @ _Robj(spec["rot"], Rb).to_4x4()
                      @ Matrix.Diagonal((fit, fit, fit, 1.0)))
    t.data.materials.append(_emit_text_mat((0.2, 0.45, 1.0)))
    return t


# ── public API ──────────────────────────────────────────────────────────────────────────────────
def clear_scene():
    for o in list(bpy.data.objects):
        bpy.data.objects.remove(o, do_unlink=True)
    for c in list(bpy.data.pointclouds):
        bpy.data.pointclouds.remove(c)


def set_camera(pos, dist, yaw, pitch):
    """Place CAM.martin (FOV π/4, Track-To an empty at `pos`), lock the viewport to it."""
    for nm in ("CAM.martin", "LOOK.bridge"):
        o = bpy.data.objects.get(nm)
        if o:
            bpy.data.objects.remove(o, do_unlink=True)
    look = bpy.data.objects.new("LOOK.bridge", None)
    bpy.context.collection.objects.link(look)
    look.location = m2b(pos)
    mw = Vector((dist * cos(pitch) * cos(yaw), dist * sin(pitch), dist * cos(pitch) * sin(yaw)))
    cam = bpy.data.cameras.new("CAM.martin")
    cam.sensor_fit = "VERTICAL"
    cam.lens_unit = "FOV"
    cam.angle = radians(45)              # martin FRAC_PI_4 vertical
    cob = bpy.data.objects.new("CAM.martin", cam)
    bpy.context.collection.objects.link(cob)
    cob.location = m2b((pos[0] + mw[0], pos[1] + mw[1], pos[2] + mw[2]))
    tt = cob.constraints.new("TRACK_TO")
    tt.target = look
    tt.track_axis = "TRACK_NEGATIVE_Z"
    tt.up_axis = "UP_Y"
    bpy.context.scene.camera = cob
    for area in bpy.context.screen.areas:
        if area.type == "VIEW_3D":
            sd = area.spaces.active
            sd.region_3d.view_perspective = "CAMERA"
            sd.lock_camera = True
            sd.shading.type = "MATERIAL"


def _setup_world():
    sc = bpy.context.scene
    sc.render.resolution_x, sc.render.resolution_y = 854, 480   # 16:9 to match martin framing
    try:
        sc.render.engine = "BLENDER_EEVEE_NEXT"
    except Exception:
        pass
    sc.world.use_nodes = True
    sc.world.node_tree.nodes["Background"].inputs[0].default_value = (0.03, 0.04, 0.06, 1)
    if not bpy.data.objects.get("Sun"):
        L = bpy.data.lights.new("Sun", "SUN")
        L.energy = 3.0
        so = bpy.data.objects.new("Sun", L)
        bpy.context.collection.objects.link(so)
        so.rotation_euler = (0.6, 0.2, 0.4)


def bridge_import(show_path, cam_anchor=None, backdrop=None, splat_n=10000):
    """Build the `.show` `[stage]` in Blender (props tagged with their source line) + set the camera.

    cam_anchor: pick the `[camera]` keyframe whose `t=` contains this (e.g. 'climax'); default = first.
    backdrop:   a `.ply` name to load as a static city point cloud (for reel/temporal shows).
    """
    clear_scene()
    _setup_world()
    if backdrop:
        _spawn_splat(dict(name=backdrop, pos=[0, 0, 0], scale=[1, 1, 1], rot=[0, 0, 0]), splat_n * 4)
    n = 0
    for spec in parse_stage(show_path):
        if spec["kind"] == "splat":
            ob = _spawn_splat(spec, splat_n)
        elif spec["kind"] == "mesh":
            ob = _spawn_mesh(spec)
        elif spec["kind"] == "text":
            ob = _spawn_text(spec)
        else:
            ob = None
        if ob:
            ob["_show_line"] = spec["line"]
            ob["_show_kind"] = spec["kind"]
            ob["_has_travel"] = spec["has_travel"]
            n += 1
    cams = parse_camera(show_path)
    cam = next((c for c in cams if cam_anchor and cam_anchor in str(c["t"])), cams[0] if cams else None)
    if cam:
        set_camera(cam["pos"], cam["dist"], cam["yaw"], cam["pitch"])
    print(f"bridge_import: {n} props from {show_path}" + (f" + backdrop {backdrop}" if backdrop else ""))


def read_camera():
    """Read CAM.martin -> martin orbit params (pos target, dist, yaw, pitch)."""
    cam = bpy.data.objects.get("CAM.martin")
    look = bpy.data.objects.get("LOOK.bridge")
    if not cam or not look:
        return None
    P = Vector(b2m(cam.matrix_world.translation))
    T = Vector(b2m(look.matrix_world.translation))
    d = P - T
    dist = d.length
    u = d / max(dist, 1e-6)
    return dict(pos=tuple(round(x, 3) for x in T), dist=round(dist, 3),
                yaw=round(math.atan2(u.z, u.x), 4), pitch=round(math.asin(max(-1, min(1, u.y))), 4))


def bridge_export(show_path, write=True):
    """Read every tagged PROP back, patch ONLY @pos/*scale/rot on its source line (tokens preserved).

    `travel:` props update the travel TARGET (their rest pos). Prints the camera line too. write=False
    just returns the patched lines (dry run).
    """
    path = show_path if os.path.isabs(show_path) else os.path.join(ROOT, show_path)
    lines = open(path).read().split("\n")
    for ob in bpy.data.objects:
        if "_show_line" not in ob.keys():
            continue
        li = ob["_show_line"]
        kind = ob["_show_kind"]
        Rbase = Euler((radians(90), 0, 0)).to_matrix() if kind == "text" else Matrix.Identity(3)
        pos = b2m(ob.matrix_world.translation)
        rot = _rot_to_martin(ob, Rbase)
        sc = round(max(ob.scale), 3) if kind != "splat" else None  # splats: per-axis baked, leave *scale
        ln = lines[li]
        # @pos (or travel: target for a travel prop)
        if ob["_has_travel"]:
            ln = re.sub(r"travel:[^\s]*?(@[^\s,]+)",
                        lambda m: "travel:%g,%g,%g%s" % (pos[0], pos[1], pos[2], m.group(1)), ln)
        else:
            ln = re.sub(r"@[-\d.][^\s]*", "@%g,%g,%g" % (pos[0], pos[1], pos[2]), ln, count=1)
        # rot (replace existing `rot a,b,c`; add one for meshes/text that gained a rotation)
        if any(abs(a) > 0.05 for a in rot):
            if re.search(r"\brot\s+[-\d.]", ln):
                ln = re.sub(r"\brot\s+[-\d.][^\s]*", "rot %g,%g,%g" % rot, ln)
            else:
                ln = re.sub(r"(@[^\s]+\s+\*[^\s]+)", r"\1  rot %g,%g,%g" % rot, ln, count=1)
        if sc is not None and re.search(r"\*[-\d.]", ln):
            ln = re.sub(r"\*[-\d.][^\s]*", "*%g" % sc, ln, count=1)
        lines[li] = ln
    cam = read_camera()
    if write:
        open(path, "w").write("\n".join(lines))
        print("bridge_export: wrote", show_path)
    if cam:
        print("camera: t=...  pos=%g,%g,%g  dist=%g  yaw=%.4f  pitch=%.4f" %
              (*cam["pos"], cam["dist"], cam["yaw"], cam["pitch"]))
    return cam


def bridge_shot(out="/tmp/blender_scene.png", render=True):
    """Save a shot to disk so it can be delivered (`DISPLAY=:0 xdg-open <out>`)."""
    sc = bpy.context.scene
    if render:
        sc.render.filepath = out
        bpy.ops.render.render(write_still=True)
    else:
        for area in bpy.context.screen.areas:
            if area.type == "VIEW_3D":
                for region in area.regions:
                    if region.type == "WINDOW":
                        with bpy.context.temp_override(area=area, region=region):
                            bpy.ops.screen.screenshot_area(filepath=out)
    print("bridge_shot:", out)
