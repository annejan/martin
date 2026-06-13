"""Bake assets/bitterbal.obj into a clean solid-brown GLB.

Run headless:
    blender-5.1 -b -P pipeline/bitterbal_glb.py

Imports the raw blob mesh, drops the OBJ's grey/junk materials, assigns a
single deep-fried-brown Principled BSDF, shades it smooth, and exports a
single-material GLB to assets/bitterbal.glb.
"""

import bpy
import os

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "assets", "bitterbal.obj")
DST = os.path.join(ROOT, "assets", "bitterbal.glb")

# deep-fried-snack brown (from bitterbal.mtl Kd), linear RGB
BROWN = (0.45, 0.26, 0.11, 1.0)

# --- clean slate ---
bpy.ops.wm.read_factory_settings(use_empty=True)

# --- import ---
bpy.ops.wm.obj_import(filepath=SRC)
objs = [o for o in bpy.context.scene.objects if o.type == "MESH"]
assert objs, "no mesh imported"

# --- single brown material ---
mat = bpy.data.materials.new("bitterbal_brown")
mat.use_nodes = True
bsdf = mat.node_tree.nodes["Principled BSDF"]
bsdf.inputs["Base Color"].default_value = BROWN
bsdf.inputs["Roughness"].default_value = 0.85   # matte, deep-fried crust
bsdf.inputs["Metallic"].default_value = 0.0

for ob in objs:
    ob.data.materials.clear()
    ob.data.materials.append(mat)
    # solid round blob -> smooth shading
    for poly in ob.data.polygons:
        poly.use_smooth = True

# --- export ---
bpy.ops.export_scene.gltf(
    filepath=DST,
    export_format="GLB",
    use_selection=False,
    export_apply=True,
    export_yup=True,
)
print("wrote", DST)
