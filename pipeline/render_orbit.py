# ============================================================================
# render_orbit.py — Blender headless: a textured mesh -> orbital RGBA renders +
# a transforms.json with the EXACT camera poses (so the splat trainer needs no
# COLMAP — the poses are known, not solved). Output is a clean synthetic 3DGS
# dataset; mesh-splat.sh then trains it with Brush.
#
# Run (via mesh-splat.sh, or directly):
#   blender-5.0 -b -P render_orbit.py -- <mesh> <out_dir> [views] [res]
#
#   <mesh>     .obj / .dae / .stl / .ply / .glb|.gltf / .fbx
#   <out_dir>  dataset dir to write (images/ + transforms.json)
#   views      number of camera viewpoints on a Fibonacci sphere (default 150)
#   res        square render resolution in px (default 800)
#
# The model is centred + scaled to ~2 units so the camera distance is uniform.
# Lit with an even world ambient + a soft key sun so material/texture colours
# read true (we want albedo-ish coverage, not dramatic shadows baked into the
# splat). Film is transparent (RGBA) → the trainer fits only the object, no
# background floaters.
# ============================================================================
import bpy
import sys
import json
import math
from mathutils import Vector, Matrix

# ---- args after the `--` separator -----------------------------------------
argv = sys.argv
argv = argv[argv.index("--") + 1:] if "--" in argv else []
if len(argv) < 2:
    print("usage: blender -b -P render_orbit.py -- <mesh> <out_dir> [views] [res]")
    sys.exit(1)
mesh_path = argv[0]
out_dir = argv[1]
n_views = int(argv[2]) if len(argv) > 2 else 150
res = int(argv[3]) if len(argv) > 3 else 800

import os
img_dir = os.path.join(out_dir, "images")
os.makedirs(img_dir, exist_ok=True)


# ---- start from an empty scene ---------------------------------------------
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.delete()


# Formats Blender 5.0 imports natively. Anything else (notably COLLADA .dae, which
# Blender 5.0 dropped) is converted to .glb with `assimp` first — assimp reads ~everything and
# .glb keeps materials/UVs/textures.
import subprocess
import tempfile

NATIVE = {
    ".obj": lambda p: bpy.ops.wm.obj_import(filepath=p),
    ".ply": lambda p: bpy.ops.wm.ply_import(filepath=p),
    ".stl": lambda p: bpy.ops.wm.stl_import(filepath=p),
    ".fbx": lambda p: bpy.ops.import_scene.fbx(filepath=p),
    ".glb": lambda p: bpy.ops.import_scene.gltf(filepath=p),
    ".gltf": lambda p: bpy.ops.import_scene.gltf(filepath=p),
}


def import_mesh(path):
    ext = os.path.splitext(path)[1].lower()
    if ext not in NATIVE:
        glb = os.path.join(tempfile.gettempdir(), "render_orbit_convert.glb")
        print(f"converting {ext} -> .glb via assimp")
        subprocess.run(["assimp", "export", path, glb], check=True)
        path, ext = glb, ".glb"
    NATIVE[ext](path)


import_mesh(mesh_path)
objs = [o for o in bpy.context.scene.objects if o.type == "MESH"]
if not objs:
    print("ERROR: no mesh objects imported")
    sys.exit(1)


# ---- centre + scale to ~2 units across (uniform camera framing) ------------
lo = Vector((1e18, 1e18, 1e18))
hi = Vector((-1e18, -1e18, -1e18))
for ob in objs:
    for corner in ob.bound_box:
        w = ob.matrix_world @ Vector(corner)
        lo = Vector((min(lo[i], w[i]) for i in range(3)))
        hi = Vector((max(hi[i], w[i]) for i in range(3)))
center = (lo + hi) * 0.5
size = max((hi - lo)[i] for i in range(3)) or 1.0
scale = 2.0 / size
fix = Matrix.Scale(scale, 4) @ Matrix.Translation(-center)
for ob in objs:
    ob.matrix_world = fix @ ob.matrix_world
radius = 1.0  # half of the ~2-unit normalized model

# The thinnest bounding-box axis is the object's "flat" normal (a PCB's board plane). A uniform
# camera sphere sees a flat object mostly edge-on → mush + depth ambiguity, so we bias the cameras
# toward the two big FACES (±thin axis) where the detail is. FACE_BIAS=0 reverts to a plain sphere.
dims = [(hi - lo)[i] for i in range(3)]
thin = dims.index(min(dims))
face_axis = Vector((1.0 if k == thin else 0.0 for k in range(3)))
face_bias = float(os.environ.get("FACE_BIAS", "1.3"))


# ---- even lighting: world ambient + a soft key sun -------------------------
world = bpy.data.worlds.new("W") if not bpy.context.scene.world else bpy.context.scene.world
bpy.context.scene.world = world
world.use_nodes = True
bg = world.node_tree.nodes.get("Background")
if bg:
    bg.inputs[0].default_value = (1.0, 1.0, 1.0, 1.0)
    bg.inputs[1].default_value = 0.7  # ambient strength
for ang, energy in (((0.6, 0.2, 0.9), 3.0), ((-0.7, -0.3, 0.4), 1.5)):
    ld = bpy.data.lights.new("Sun", "SUN")
    ld.energy = energy
    so = bpy.data.objects.new("Sun", ld)
    bpy.context.collection.objects.link(so)
    so.rotation_euler = ang


# ---- camera ----------------------------------------------------------------
cam_data = bpy.data.cameras.new("Cam")
cam_data.lens = 50.0  # mm on the default 36mm sensor → ~39.6° horizontal FOV
cam = bpy.data.objects.new("Cam", cam_data)
bpy.context.collection.objects.link(cam)
bpy.context.scene.camera = cam
cam_dist = radius * 2.8

scene = bpy.context.scene
# Cycles on CPU: renders headless with no GL/display (EEVEE needs a GL context and segfaults over
# SSH / without an X server), is deterministic, uses every core, and lights the model cleanly.
# SAMPLES env tunes quality vs. speed (denoised, so low counts stay clean).
scene.render.engine = "CYCLES"
scene.cycles.device = "CPU"
scene.cycles.samples = int(os.environ.get("SAMPLES", "48"))
try:
    scene.cycles.use_denoising = True
    bpy.context.view_layer.cycles.use_denoising = True
except Exception as e:
    print(f"denoise unavailable ({e})")
scene.render.resolution_x = res
scene.render.resolution_y = res
scene.render.film_transparent = True
scene.render.image_settings.file_format = "PNG"
scene.render.image_settings.color_mode = "RGBA"


def look_at_matrix(eye, target, up=Vector((0.0, 0.0, 1.0))):
    """Camera-to-world (OpenGL convention: camera looks down -Z, +Y up)."""
    forward = (target - eye).normalized()
    right = forward.cross(up).normalized()
    if right.length < 1e-6:  # looking straight up/down → pick another up
        right = forward.cross(Vector((0.0, 1.0, 0.0))).normalized()
    cam_up = right.cross(forward)
    m = Matrix.Identity(4)
    m.col[0][:3] = right        # +X
    m.col[1][:3] = cam_up       # +Y
    m.col[2][:3] = -forward     # +Z (camera looks down -Z)
    m.col[3][:3] = eye
    return m


# ---- render N views on a Fibonacci sphere ----------------------------------
frames = []
ga = math.pi * (3.0 - math.sqrt(5.0))  # golden angle
for i in range(n_views):
    y = 1.0 - (i + 0.5) / n_views * 2.0  # -1..1
    r = math.sqrt(max(0.0, 1.0 - y * y))
    theta = ga * i
    direction = Vector((math.cos(theta) * r, y, math.sin(theta) * r))
    # pull each camera toward the nearest face (along the thin axis) so the faces get dense,
    # high-parallax coverage; near-edge cameras (small dot) stay put for some thickness coverage.
    direction = (direction + face_bias * direction.dot(face_axis) * face_axis).normalized()
    eye = direction * cam_dist
    cam.matrix_world = look_at_matrix(eye, Vector((0.0, 0.0, 0.0)))

    name = f"r_{i:03d}.png"
    scene.render.filepath = os.path.join(img_dir, name)
    bpy.ops.render.render(write_still=True)

    frames.append({
        "file_path": f"./images/{name}",
        "transform_matrix": [list(row) for row in cam.matrix_world],
    })

# ---- write transforms.json (NeRF/nerfstudio synthetic) ---------------------
angle_x = cam_data.angle_x
fl = 0.5 * res / math.tan(0.5 * angle_x)
meta = {
    "camera_angle_x": angle_x,
    "fl_x": fl, "fl_y": fl,
    "cx": res / 2.0, "cy": res / 2.0,
    "w": res, "h": res,
    "aabb_scale": 1,
    "frames": frames,
}
with open(os.path.join(out_dir, "transforms.json"), "w") as f:
    json.dump(meta, f, indent=2)

print(f"render_orbit: {n_views} views @ {res}px -> {out_dir}")
