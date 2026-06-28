# SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
# SPDX-License-Identifier: MIT
"""Author martin camera waypoints in Blender's viewport, for the aerial-city captures.

The idea: martin's live window is flaky on RADV, and authoring a flowing camera by hand in a
text `[camera]` track is tedious. Instead, fly Blender's *free viewport* over a lightweight colored
PROXY of the splat cloud, read the viewport pose, and emit a martin `[camera]` line. Confirm framing
by rendering that pose in martin (real colors) — the proxy is a ROUGH guide, not pixel-1:1.

VERIFIED 2026-06-28 (a long session): the proxy below matches martin's render with NO mirror. An
earlier "X-mirror" was self-inflicted (I wrongly X-negated the maps); the STANDARD Bevy↔Blender map
is correct. Confirmed against a Luigi proxy vs martin-Luigi (cap "L" reads forward in both).

COORDINATE MAPS (do NOT X-negate):
    martin (Bevy, Y-up) -> blender (Z-up):   m2b(x,y,z) = (x, -z,  y)     # det +1, standard
    blender -> martin (read a viewport pose): b2m(x,y,z) = (x,  z, -y)     # inverse

martin's compose city transform (what the proxy must match):
    cloud positions = morph::normalize_to  -> centroid(MEAN)-center + scale so p90-radius -> 1
    entity rotation = cloud_base_rotation   -> rotate_X(pi)  == (x,-y,-z)   (det +1)
    => proxy = m2b( base_rot( normalize(raw) ) )

GOTCHAS learned the hard way:
  * A `[camera]` needs >=1 TIMED keyframe to apply (fix 1435d22: `is_timed`). One key = held pose.
  * martin's camera is ALWAYS upright (look_at(target,+Y), no roll) -> a rolled/Dutch-angle Blender
    view can't transfer (horizon stays horizontal in martin).
  * EXTREME pitch (near +-pi/2, looking ~straight down) is unreliable: yaw is gimbal-lock-unstable
    and the coarse proxy hides what you're aiming at. Keep waypoints to MODERATE pitch (|pitch| < ~1.0,
    the dramatic low-skyline range ~-0.7 that read perfectly). Preview each in martin BEFORE logging.
  * The free viewport's FOV is not martin's (pi/4 vertical); framing is approximate at the edges.

Usage:
    python pipeline/blender_cities_bridge.py build  <in.ply> <out_proxy.ply> [--pts 35000] [--no-flip]
        Build a colored proxy. --no-flip skips the base X-180 (use it for assets martin renders with
        `rot 180,0,0`, i.e. net-identity rotation — e.g. an upright model, not an aerial capture).
    Then paste BLENDER_SNIPPET (below) into Blender to import + set up, and call read_pose()/set_view().
"""

import sys

import numpy as np

C0 = 0.28209479177387814  # SH degree-0 constant: rgb = 0.5 + C0 * f_dc


def _read_ply(path, target_pts):
    names, n = [], 0
    with open(path, "rb") as f:
        while True:
            line = f.readline().decode("ascii", "ignore").strip()
            if line.startswith("element vertex"):
                n = int(line.split()[-1])
            elif line.startswith("property float"):
                names.append(line.split()[-1])
            elif line == "end_header":
                break
        ds = f.tell()
    props = len(names)
    arr = np.memmap(path, dtype=np.float32, mode="r", offset=ds, shape=(n, props))
    k = max(1, n // target_pts)
    sub = np.array(arr[::k], dtype=np.float64)
    xyz = sub[:, [names.index(c) for c in ("x", "y", "z")]]
    fdc = None
    if all(c in names for c in ("f_dc_0", "f_dc_1", "f_dc_2")):
        fdc = sub[:, [names.index(c) for c in ("f_dc_0", "f_dc_1", "f_dc_2")]]
    return xyz, fdc


def build_proxy(in_ply, out_ply, target_pts=35000, base_flip=True):
    """Build a colored Blender proxy that matches martin's render of `in_ply`."""
    xyz, fdc = _read_ply(in_ply, target_pts)
    xyz = xyz - xyz.mean(0)  # martin normalize_to centers on the MEAN (centroid)
    xyz = xyz / np.percentile(np.linalg.norm(xyz, axis=1), 90)  # p90-radius -> 1
    if base_flip:
        xyz = xyz * np.array([1, -1, -1])  # cloud_base_rotation = rotate_X(pi)
    bl = np.stack([xyz[:, 0], -xyz[:, 2], xyz[:, 1]], 1)  # m2b = (x,-z,y)  STANDARD, no X-flip
    if fdc is not None:
        rgb = (np.clip(0.5 + C0 * fdc, 0, 1) * 255).astype(np.uint8)
    else:
        rgb = np.full((len(bl), 3), 200, np.uint8)
    with open(out_ply, "w") as o:
        o.write(
            "ply\nformat ascii 1.0\nelement vertex %d\n"
            "property float x\nproperty float y\nproperty float z\n"
            "property uchar red\nproperty uchar green\nproperty uchar blue\nend_header\n" % len(bl)
        )
        for p, c in zip(bl, rgb):
            o.write("%.4f %.4f %.4f %d %d %d\n" % (p[0], p[1], p[2], c[0], c[1], c[2]))
    print("proxy: %d pts -> %s (base_flip=%s)" % (len(bl), out_ply, base_flip))


# Paste this into Blender (BlenderMCP execute_blender_code) to import the proxy + define the helpers.
BLENDER_SNIPPET = r'''
import bpy, math
from mathutils import Vector, Matrix

def import_proxy(path="/tmp/nyc_proxy.ply", radius=0.010):
    for nm in ['nyc_proxy']:
        if nm in bpy.data.objects: bpy.data.objects.remove(bpy.data.objects[nm], do_unlink=True)
    bpy.ops.wm.ply_import(filepath=path)
    c = bpy.context.selected_objects[0]; c.name='nyc_proxy'
    c.rotation_euler=(0,0,0); c.scale=(1,1,1); c.location=(0,0,0)   # KEEP at identity (don't transform the object!)
    bpy.ops.object.select_all(action='DESELECT'); bpy.context.view_layer.objects.active=c; c.select_set(True)
    bpy.ops.object.convert(target='POINTCLOUD'); c=bpy.context.view_layer.objects.active
    try: c.data.points.foreach_set('radius',[radius]*len(c.data.points))
    except Exception: pass
    m=bpy.data.materials.get('proxy_col') or bpy.data.materials.new('proxy_col'); m.use_nodes=True; nt=m.node_tree
    for nd in list(nt.nodes): nt.nodes.remove(nd)
    a=nt.nodes.new('ShaderNodeAttribute'); a.attribute_name='Col'
    e=nt.nodes.new('ShaderNodeEmission'); o=nt.nodes.new('ShaderNodeOutputMaterial')
    nt.links.new(a.outputs['Color'],e.inputs['Color']); nt.links.new(e.outputs['Emission'],o.inputs['Surface'])
    c.data.materials.clear(); c.data.materials.append(m)
    bpy.ops.object.select_all(action='DESELECT'); bpy.context.view_layer.objects.active=None

def _sp(): return next(s for ar in bpy.context.screen.areas if ar.type=='VIEW_3D' for s in ar.spaces if s.type=='VIEW_3D')
def b2m(v): return Vector((v.x, v.z, -v.y))   # blender -> martin (standard)
def m2b(v): return Vector((v.x, -v.z, v.y))

def read_pose():
    """Read the FREE viewport as a martin [camera] line."""
    sp=_sp(); r=sp.region_3d
    if r.view_perspective=='CAMERA': r.view_perspective='PERSP'   # never read the camera object
    fwd=r.view_rotation@Vector((0,0,-1)); eye=r.view_location-fwd*r.view_distance
    P=b2m(eye); T=b2m(r.view_location); d=P-T; dist=d.length; u=d/max(dist,1e-6)
    yaw=math.atan2(u.z,u.x); pitch=math.asin(max(-1,min(1,u.y)))
    print("t=0  pos=%.3f,%.3f,%.3f  dist=%.3f  yaw=%.4f  pitch=%.4f"%(T.x,T.y,T.z,dist,yaw,pitch))

def set_view(Tm, dist, yaw, pitch):
    """Put the viewport AT a martin pose (to re-inspect a logged waypoint)."""
    sp=_sp(); sp.shading.type='MATERIAL'; sp.overlay.show_overlays=True
    Tm=Vector(Tm)
    off=Vector((math.cos(pitch)*math.cos(yaw), math.sin(pitch), math.cos(pitch)*math.sin(yaw)))*dist
    T=m2b(Tm); P=m2b(Tm+off)
    fwd=(T-P).normalized(); up=Vector((0,0,1))
    right=fwd.cross(up).normalized(); tup=right.cross(fwd).normalized()
    r=sp.region_3d; r.view_perspective='PERSP'; r.view_location=T; r.view_distance=(P-T).length
    r.view_rotation=Matrix((right,tup,-fwd)).transposed().to_quaternion()

import_proxy()
print("ready: navigate the viewport, then read_pose(). DO NOT press Numpad-0 (locks to camera).")
'''


if __name__ == "__main__":
    if len(sys.argv) >= 4 and sys.argv[1] == "build":
        pts = 35000
        flip = "--no-flip" not in sys.argv
        if "--pts" in sys.argv:
            pts = int(sys.argv[sys.argv.index("--pts") + 1])
        build_proxy(sys.argv[2], sys.argv[3], pts, flip)
    else:
        print(__doc__)
        print("\n--- BLENDER_SNIPPET (paste into Blender) ---\n" + BLENDER_SNIPPET)
