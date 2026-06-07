#!/usr/bin/env python3
"""Build the deFEEST 3D logo from the official vector `defeest.svg`.

End-to-end (needs `openscad` on PATH; no Blender required):
  1. split defeest.svg into its three colour layers — yellow base ellipse,
     blue ellipse, yellow letters (path order: [0]=base, [1]=blue, [2:]=text);
  2. `openscad` extrude each, centred on z=0 (so the logo is **mirror-symmetric**,
     reading from the front AND the back), in a coin/badge layout: the yellow RIM
     (the ellipse minus the blue field) and the LETTERS stand proud at the SAME
     thickness, while the BLUE field is thinner so it sits INSET (recessed). Only
     the yellow rim's OUTER edge gets a very subtle soft bevel (minkowski on the
     convex ellipse, then the blue field is punched out with a crisp inner edge);
     the letters stay crisp;
  3. assemble the three extrusions into ONE glTF (`defeest.glb`, canonical) and
     Collada (`defeest.dae`, what the show loads via `mesh:`), each layer its
     own coloured material.

    python3 pipeline/svg_extrude_logo.py      # regenerates defeest.glb + .dae
"""
import struct, json, os, re, subprocess, tempfile
import xml.etree.ElementTree as ET

HERE = os.path.dirname(os.path.abspath(__file__))
ASSETS = os.path.join(os.path.dirname(HERE), "assets")   # script lives in pipeline/, assets sit in assets/
SVG = os.path.join(ASSETS, "defeest.svg")
OUT = os.path.join(ASSETS, "defeest")
SVG_NS = "http://www.w3.org/2000/svg"
ET.register_namespace("", SVG_NS)   # default ns (no ns0: prefixes — openscad needs clean SVG)
YELLOW = (1.0, 0.9608, 0.4274)        # SVG fill rgb(100%,96.08%,42.74%)
BLUE   = (0.1098, 0.3882, 0.6863)     # SVG fill rgb(10.98%,38.82%,68.63%)
# Coin/badge build: the yellow RIM (the ellipse minus the blue field) and the
# LETTERS stand proud at the SAME thickness; the BLUE field is thinner so it sits
# INSET (recessed) between them. Everything centred on z=0 → mirror-symmetric.
# Only the yellow rim gets a soft bevel — its outer edge is the coin's rim.
RIM_T = LETTER_T = 3.0     # rim + letters: same thickness, the proud top level
BLUE_T = 2.0              # thinner → recessed / inset between rim and letters
RIM_BEVEL = 0.13          # soft rim radius, as a fraction of RIM_T (the sweet spot; < 0.5)

# ---- 1. split defeest.svg into base / blue / text path groups ----
root = ET.parse(SVG).getroot()
W, H, VB = root.get("width"), root.get("height"), root.get("viewBox")
paths = [p.get("d") for p in root.iter(f"{{{SVG_NS}}}path")
         if "fill:rgb" in (p.get("style") or "")]
groups = {"base": [paths[0]], "blue": [paths[1]], "text": paths[2:]}

tmp = tempfile.mkdtemp(prefix="defeest_logo_")
def write_svg(key):
    svg = ET.Element(f"{{{SVG_NS}}}svg", {"width": W, "height": H, "viewBox": VB, "version": "1.1"})
    g = ET.SubElement(svg, f"{{{SVG_NS}}}g")
    for d in groups[key]:
        ET.SubElement(g, f"{{{SVG_NS}}}path", {"d": d, "style": "fill:#000;stroke:none"})
    path = os.path.join(tmp, f"{key}.svg")
    ET.ElementTree(svg).write(path, xml_declaration=True, encoding="utf-8")
    return path

base_svg, blue_svg, text_svg = write_svg("base"), write_svg("blue"), write_svg("text")
def imp(p): return f'import("{p}", center=false)'

# Bodies, all centred on z=0. The yellow rim rounds only its OUTER edge: minkowski
# on the **convex full ellipse** (fast), THEN punch the blue field out — a sharp
# inner edge, which is right where it meets the recessed blue. Letters + blue are
# plain extrudes (crisp). r is tiny → a barely-there soft edge.
r = RIM_BEVEL * RIM_T
rim_body = (
    f'difference() {{\n'
    f'  minkowski() {{ linear_extrude(height={RIM_T - 2*r:.4f}, center=true) '
    f'offset(delta={-r:.4f}) {imp(base_svg)}; sphere(r={r:.4f}, $fn=12); }}\n'
    f'  linear_extrude(height={RIM_T * 2:.4f}, center=true) {imp(blue_svg)};\n'
    f'}}')
BUILD = [
    ("Yellow_rim", YELLOW, rim_body),
    ("Letters",    YELLOW, f'linear_extrude(height={LETTER_T}, center=true) {imp(text_svg)};'),
    ("Blue",       BLUE,   f'linear_extrude(height={BLUE_T}, center=true) {imp(blue_svg)};'),
]

# ---- 2. openscad: run each body to an STL ----
def extrude(name, body):
    scad = os.path.join(tmp, f"{name}.scad")
    stl = os.path.join(tmp, f"{name}.stl")
    with open(scad, "w") as f:
        f.write(f'$fn=96;\n{body}\n')
    subprocess.run(["openscad", "-o", stl, scad], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return stl

def read_stl(path):
    """ASCII STL (OpenSCAD default) -> (positions[3T], normals[3T]) triangle soup,
    with a freshly computed face normal per triangle."""
    verts = re.findall(r"vertex\s+(\S+)\s+(\S+)\s+(\S+)", open(path).read())
    fv = [(float(a), float(b), float(c)) for a, b, c in verts]
    pos, nrm = [], []
    for i in range(0, len(fv), 3):
        tri = fv[i:i+3]
        ux, uy, uz = (tri[1][j] - tri[0][j] for j in range(3))
        vx, vy, vz = (tri[2][j] - tri[0][j] for j in range(3))
        cx, cy, cz = uy*vz - uz*vy, uz*vx - ux*vz, ux*vy - uy*vx
        L = (cx*cx + cy*cy + cz*cz) ** 0.5 or 1.0
        fn = (cx/L, cy/L, cz/L)
        for v in tri:
            pos.append(v); nrm.append(fn)
    return pos, nrm

layers = [(name, rgb, *read_stl(extrude(name, body))) for name, rgb, body in BUILD]

# ---- 3a. glTF (.glb, canonical) ----
bin_ = bytearray()
bufviews, accessors, materials, meshes, nodes = [], [], [], [], []
def f32(vals):
    off = len(bin_)
    for v in vals:
        bin_.extend(struct.pack("<3f", *v))
    bufviews.append({"buffer": 0, "byteOffset": off, "byteLength": len(vals)*12})
    return len(bufviews) - 1
def srgb_to_lin(c):
    return c/12.92 if c <= 0.04045 else ((c+0.055)/1.055) ** 2.4
for name, rgb, pos, nrm in layers:
    bvp, bvn = f32(pos), f32(nrm)
    xs=[p[0] for p in pos]; ys=[p[1] for p in pos]; zs=[p[2] for p in pos]
    ap = len(accessors)
    accessors.append({"bufferView": bvp, "componentType": 5126, "count": len(pos),
                      "type": "VEC3", "min": [min(xs),min(ys),min(zs)], "max": [max(xs),max(ys),max(zs)]})
    an = len(accessors)
    accessors.append({"bufferView": bvn, "componentType": 5126, "count": len(nrm), "type": "VEC3"})
    mat = len(materials)
    materials.append({"name": name, "pbrMetallicRoughness": {
        "baseColorFactor": [srgb_to_lin(c) for c in rgb] + [1.0],
        "metallicFactor": 0.0, "roughnessFactor": 0.8}})
    meshes.append({"name": name, "primitives": [{"attributes": {"POSITION": ap, "NORMAL": an}, "material": mat}]})
    nodes.append({"name": name, "mesh": len(meshes)-1})
while len(bin_) % 4: bin_.append(0)
gltf = {"asset": {"version": "2.0", "generator": "svg_extrude_logo"}, "scene": 0,
        "scenes": [{"nodes": list(range(len(nodes)))}], "nodes": nodes, "meshes": meshes,
        "materials": materials, "accessors": accessors, "bufferViews": bufviews,
        "buffers": [{"byteLength": len(bin_)}]}
js = json.dumps(gltf, separators=(",", ":")).encode()
while len(js) % 4: js += b" "
with open(OUT + ".glb", "wb") as f:
    f.write(struct.pack("<III", 0x46546C67, 2, 12 + 8 + len(js) + 8 + len(bin_)))
    f.write(struct.pack("<II", len(js), 0x4E4F534A)); f.write(js)
    f.write(struct.pack("<II", len(bin_), 0x004E4942)); f.write(bin_)

# ---- 3b. Collada (.dae, what the show loads via mesh:) ----
def farr(vals):
    return " ".join("%.5g" % x for v in vals for x in v)
fx, mats, geos, scene_nodes = [], [], [], []
for name, rgb, pos, nrm in layers:
    fx.append(f'''    <effect id="{name}-fx"><profile_COMMON><technique sid="common">
      <lambert><diffuse><color>{rgb[0]:.5g} {rgb[1]:.5g} {rgb[2]:.5g} 1</color></diffuse></lambert>
    </technique></profile_COMMON></effect>''')
    mats.append(f'    <material id="{name}-mat"><instance_effect url="#{name}-fx"/></material>')
    V = len(pos); T = V // 3
    p_idx = " ".join(str(i) for i in range(V))
    geos.append(f'''    <geometry id="{name}-geo"><mesh>
      <source id="{name}-pos"><float_array id="{name}-pos-a" count="{V*3}">{farr(pos)}</float_array>
        <technique_common><accessor source="#{name}-pos-a" count="{V}" stride="3">
          <param name="X" type="float"/><param name="Y" type="float"/><param name="Z" type="float"/></accessor></technique_common></source>
      <source id="{name}-nrm"><float_array id="{name}-nrm-a" count="{V*3}">{farr(nrm)}</float_array>
        <technique_common><accessor source="#{name}-nrm-a" count="{V}" stride="3">
          <param name="X" type="float"/><param name="Y" type="float"/><param name="Z" type="float"/></accessor></technique_common></source>
      <vertices id="{name}-v"><input semantic="POSITION" source="#{name}-pos"/></vertices>
      <triangles material="{name}-mat" count="{T}">
        <input semantic="VERTEX" source="#{name}-v" offset="0"/>
        <input semantic="NORMAL" source="#{name}-nrm" offset="0"/>
        <p>{p_idx}</p>
      </triangles></mesh></geometry>''')
    scene_nodes.append(f'''      <node id="{name}" name="{name}" type="NODE">
        <instance_geometry url="#{name}-geo"><bind_material><technique_common>
          <instance_material symbol="{name}-mat" target="#{name}-mat"/></technique_common></bind_material></instance_geometry></node>''')
dae = f'''<?xml version="1.0" encoding="utf-8"?>
<COLLADA xmlns="http://www.collada.org/2005/11/COLLADASchema" version="1.4.1">
  <asset><up_axis>Z_UP</up_axis></asset>
  <library_effects>
{chr(10).join(fx)}
  </library_effects>
  <library_materials>
{chr(10).join(mats)}
  </library_materials>
  <library_geometries>
{chr(10).join(geos)}
  </library_geometries>
  <library_visual_scenes><visual_scene id="Scene" name="Scene">
{chr(10).join(scene_nodes)}
  </visual_scene></library_visual_scenes>
  <scene><instance_visual_scene url="#Scene"/></scene>
</COLLADA>
'''
with open(OUT + ".dae", "w") as f:
    f.write(dae)

import shutil
shutil.rmtree(tmp, ignore_errors=True)
tris = sum(len(p)//3 for _, _, p, _ in layers)
print(f"wrote {OUT}.glb + .dae from defeest.svg — {len(layers)} layers, {tris} tris")
