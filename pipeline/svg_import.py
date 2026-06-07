#!/usr/bin/env python3
"""Import any flat SVG into a 3D mesh (`.glb` + `.dae`) for martin — a generic,
engine-level "easy asset import" (the flat-PNG path already exists as `image:`).

What it does:
  * groups the SVG's filled paths **by fill colour** (one material per colour);
  * `openscad linear_extrude(center=true)` each colour group to its own depth,
    stepped by **paint order** (the foreground / last-drawn colour is thickest),
    so overlapping colours don't share a cap plane (which would blend the splats
    into mush) and every layer stays centred on z=0 → **mirror-symmetric**
    (reads from the front AND the back);
  * assembles the groups into one glTF + Collada, each its own coloured material.

Depths auto-scale to the artwork, so it works on any logo regardless of size.
Limitations: only **filled** paths (no strokes / gradients / embedded bitmaps);
same-colour regions merge into one layer (can't be split by depth — that's what
the deFEEST-specific `svg_extrude_logo.py` is for). Messy SVGs (transforms,
`<use>`, text-as-font) → pass `--clean` to normalise with Inkscape first.

Usage:
  pipeline/svg_import.py logo.svg                 # -> assets/logo.glb + .dae
  pipeline/svg_import.py logo.svg -o out/name     # -> out/name.glb + .dae
  pipeline/svg_import.py logo.svg --depth 0.08    # thicker (frac of width)
  pipeline/svg_import.py logo.svg --uniform       # every colour the same depth
  pipeline/svg_import.py logo.svg --clean         # Inkscape object-to-path first
"""
import argparse, json, os, re, struct, subprocess, sys, tempfile
import xml.etree.ElementTree as ET

SVG_NS = "http://www.w3.org/2000/svg"
ET.register_namespace("", SVG_NS)
PX_TO_MM = 25.4 / 72.0   # OpenSCAD imports SVG user units at 72 dpi


def colour_to_rgb(fill):
    """A CSS colour string -> (r,g,b) in 0..1, or None (none/named/unknown)."""
    fill = (fill or "").strip()
    if not fill or fill.lower() in ("none", "transparent"):
        return None
    m = re.match(r"rgb\(([^)]+)\)", fill)
    if m:
        parts = [p.strip() for p in m.group(1).split(",")]
        return tuple((float(p[:-1]) / 100.0 if p.endswith("%") else float(p) / 255.0) for p in parts)
    m = re.match(r"#([0-9a-fA-F]{6})$", fill)
    if m:
        h = m.group(1)
        return tuple(int(h[i:i+2], 16) / 255.0 for i in (0, 2, 4))
    m = re.match(r"#([0-9a-fA-F]{3})$", fill)
    if m:
        h = m.group(1)
        return tuple(int(h[i]*2, 16) / 255.0 for i in (0, 1, 2))
    return None  # named colours etc. — not worth guessing


def parse_css_fills(root):
    """class name -> fill colour string, from <style> blocks (Illustrator/Inkscape
    export fills as `.st0{fill:#...}` classes rather than inline)."""
    css = {}
    for st in root.iter(f"{{{SVG_NS}}}style"):
        for m in re.finditer(r"\.([\w-]+)\s*\{([^}]*)\}", "".join(st.itertext())):
            fm = re.search(r"fill\s*:\s*([^;]+)", m.group(2))
            if fm:
                css[m.group(1)] = fm.group(1).strip()
    return css


def parse_fill(el, css):
    """A path's fill, resolved inline-style > fill attr > CSS class -> rgb or None."""
    m = re.search(r"fill\s*:\s*([^;]+)", el.get("style") or "")
    if m and (c := colour_to_rgb(m.group(1))) is not None:
        return c
    if (c := colour_to_rgb(el.get("fill"))) is not None:
        return c
    for name in (el.get("class") or "").split():
        if name in css and (c := colour_to_rgb(css[name])) is not None:
            return c
    return None


def viewbox_width(root):
    vb = root.get("viewBox")
    if vb:
        return float(vb.split()[2])
    w = root.get("width") or "100"
    return float(re.match(r"[\d.]+", w).group())


def group_by_colour(svg_path):
    """[(rgb, [d,...]), ...] in first-seen paint order; + extruded width (mm)."""
    root = ET.parse(svg_path).getroot()
    css = parse_css_fills(root)
    order, groups = [], {}
    for p in root.iter(f"{{{SVG_NS}}}path"):
        rgb = parse_fill(p, css)
        d = p.get("d")
        if rgb is None or not d:
            continue
        key = tuple(round(c, 4) for c in rgb)
        if key not in groups:
            groups[key] = []
            order.append(key)
        groups[key].append(d)
    return [(k, groups[k]) for k in order], viewbox_width(root) * PX_TO_MM


def srgb_to_lin(c):
    return c / 12.92 if c <= 0.04045 else ((c + 0.055) / 1.055) ** 2.4


def read_stl(path):
    verts = re.findall(r"vertex\s+(\S+)\s+(\S+)\s+(\S+)", open(path).read())
    fv = [(float(a), float(b), float(c)) for a, b, c in verts]
    pos, nrm = [], []
    for i in range(0, len(fv), 3):
        t = fv[i:i+3]
        ux, uy, uz = (t[1][j] - t[0][j] for j in range(3))
        vx, vy, vz = (t[2][j] - t[0][j] for j in range(3))
        cx, cy, cz = uy*vz - uz*vy, uz*vx - ux*vz, ux*vy - uy*vx
        L = (cx*cx + cy*cy + cz*cz) ** 0.5 or 1.0
        fn = (cx/L, cy/L, cz/L)
        for v in t:
            pos.append(v); nrm.append(fn)
    return pos, nrm


def extrude_group(tmp, viewW, VB, idx, dlist, thickness):
    svg = ET.Element(f"{{{SVG_NS}}}svg", {"viewBox": VB, "version": "1.1"})
    g = ET.SubElement(svg, f"{{{SVG_NS}}}g")
    for d in dlist:
        ET.SubElement(g, f"{{{SVG_NS}}}path", {"d": d, "style": "fill:#000;stroke:none"})
    spath = os.path.join(tmp, f"g{idx}.svg")
    ET.ElementTree(svg).write(spath, xml_declaration=True, encoding="utf-8")
    scad = os.path.join(tmp, f"g{idx}.scad")
    stl = os.path.join(tmp, f"g{idx}.stl")
    open(scad, "w").write(
        f'$fn=96;\nlinear_extrude(height={thickness:.5f}, center=true) import("{spath}", center=false);\n')
    subprocess.run(["openscad", "-o", stl, scad], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return read_stl(stl)


def write_glb(out, layers):
    bin_, bufviews, accessors, materials, meshes, nodes = bytearray(), [], [], [], [], []
    def f32(vals):
        off = len(bin_)
        for v in vals:
            bin_.extend(struct.pack("<3f", *v))
        bufviews.append({"buffer": 0, "byteOffset": off, "byteLength": len(vals)*12})
        return len(bufviews) - 1
    for name, rgb, pos, nrm in layers:
        bvp, bvn = f32(pos), f32(nrm)
        xs=[p[0] for p in pos]; ys=[p[1] for p in pos]; zs=[p[2] for p in pos]
        ap = len(accessors); accessors.append({"bufferView": bvp, "componentType": 5126,
            "count": len(pos), "type": "VEC3", "min": [min(xs),min(ys),min(zs)], "max": [max(xs),max(ys),max(zs)]})
        an = len(accessors); accessors.append({"bufferView": bvn, "componentType": 5126, "count": len(nrm), "type": "VEC3"})
        materials.append({"name": name, "pbrMetallicRoughness": {
            "baseColorFactor": [srgb_to_lin(c) for c in rgb] + [1.0], "metallicFactor": 0.0, "roughnessFactor": 0.8}})
        meshes.append({"name": name, "primitives": [{"attributes": {"POSITION": ap, "NORMAL": an}, "material": len(materials)-1}]})
        nodes.append({"name": name, "mesh": len(meshes)-1})
    while len(bin_) % 4: bin_.append(0)
    gltf = {"asset": {"version": "2.0", "generator": "svg_import"}, "scene": 0,
            "scenes": [{"nodes": list(range(len(nodes)))}], "nodes": nodes, "meshes": meshes,
            "materials": materials, "accessors": accessors, "bufferViews": bufviews,
            "buffers": [{"byteLength": len(bin_)}]}
    js = json.dumps(gltf, separators=(",", ":")).encode()
    while len(js) % 4: js += b" "
    with open(out + ".glb", "wb") as f:
        f.write(struct.pack("<III", 0x46546C67, 2, 12 + 8 + len(js) + 8 + len(bin_)))
        f.write(struct.pack("<II", len(js), 0x4E4F534A)); f.write(js)
        f.write(struct.pack("<II", len(bin_), 0x004E4942)); f.write(bin_)


def write_dae(out, layers):
    def farr(vals): return " ".join("%.5g" % x for v in vals for x in v)
    fx, mats, geos, sn = [], [], [], []
    for name, rgb, pos, nrm in layers:
        fx.append(f'    <effect id="{name}-fx"><profile_COMMON><technique sid="common"><lambert>'
                  f'<diffuse><color>{rgb[0]:.5g} {rgb[1]:.5g} {rgb[2]:.5g} 1</color></diffuse>'
                  f'</lambert></technique></profile_COMMON></effect>')
        mats.append(f'    <material id="{name}-mat"><instance_effect url="#{name}-fx"/></material>')
        V = len(pos); T = V // 3; p_idx = " ".join(str(i) for i in range(V))
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
        <p>{p_idx}</p></triangles></mesh></geometry>''')
        sn.append(f'''      <node id="{name}" name="{name}" type="NODE">
        <instance_geometry url="#{name}-geo"><bind_material><technique_common>
          <instance_material symbol="{name}-mat" target="#{name}-mat"/></technique_common></bind_material></instance_geometry></node>''')
    open(out + ".dae", "w").write(f'''<?xml version="1.0" encoding="utf-8"?>
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
{chr(10).join(sn)}
  </visual_scene></library_visual_scenes>
  <scene><instance_visual_scene url="#Scene"/></scene>
</COLLADA>
''')


def inkscape_clean(src, tmp):
    """Normalise a messy SVG: shapes/text/transforms -> plain filled paths."""
    dst = os.path.join(tmp, "clean.svg")
    subprocess.run(["inkscape", src, "--export-type=svg", "--export-plain-svg",
                    f"--export-filename={dst}",
                    "--actions=select-all;object-to-path;"], check=True,
                   stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    return dst


def main():
    ap = argparse.ArgumentParser(description="Import a flat SVG into a 3D mesh (.glb + .dae) for martin.")
    ap.add_argument("svg")
    ap.add_argument("-o", "--out", help="output basename (default: assets/<svg stem>)")
    ap.add_argument("--depth", type=float, default=0.05, help="max layer thickness as a fraction of artwork width (default 0.05)")
    ap.add_argument("--uniform", action="store_true", help="give every colour the same depth (no paint-order step)")
    ap.add_argument("--clean", action="store_true", help="normalise the SVG with Inkscape (object-to-path) first")
    args = ap.parse_args()

    root_dir = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    out = args.out or os.path.join(root_dir, "assets", os.path.splitext(os.path.basename(args.svg))[0])
    os.makedirs(os.path.dirname(out) or ".", exist_ok=True)

    tmp = tempfile.mkdtemp(prefix="svg_import_")
    src = inkscape_clean(args.svg, tmp) if args.clean else args.svg
    VB = ET.parse(src).getroot().get("viewBox") or f"0 0 {viewbox_width(ET.parse(src).getroot())} 100"
    groups, extW = group_by_colour(src)
    if not groups:
        sys.exit(f"{args.svg}: no filled paths found (strokes/gradients/bitmaps aren't supported)")

    n = len(groups)
    max_t = args.depth * extW
    layers = []
    for i, (rgb, dlist) in enumerate(groups):
        thickness = max_t if args.uniform else max_t * (i + 1) / n   # paint order: foreground thickest
        pos, nrm = extrude_group(tmp, extW, VB, i, dlist, thickness)
        name = "%02X%02X%02X" % tuple(round(c*255) for c in rgb)
        layers.append((f"c{i}_{name}", rgb, pos, nrm))

    write_glb(out, layers)
    write_dae(out, layers)
    import shutil; shutil.rmtree(tmp, ignore_errors=True)
    tris = sum(len(p)//3 for _, _, p, _ in layers)
    print(f"{args.svg} -> {out}.glb + .dae — {n} colour layer(s), {tris} tris")


if __name__ == "__main__":
    main()
