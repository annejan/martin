//! Splat-text: rasterize a string into flat (z=0) gaussians, so text is just another morph
//! source. Built Y-DOWN so the entity's `cloud_base_rotation` flips it upright like the
//! Y-down `.ply` splats. Pure (no Bevy/ECS) apart from the gaussian type.

use ab_glyph::{Font, FontRef, OutlineCurve, PxScale, ScaleFont, point};
use bevy_gaussian_splatting::{Gaussian3d, SphericalHarmonicCoefficients};

/// Bundled bold TTF (`include_bytes`, not the asset server — `AssetPlugin.file_path` points
/// at the .ply folder, which would break a relative font load).
static FONT: &[u8] = include_bytes!("../assets/font.ttf");

/// Single-stroke (centerline) font for true pen-write handwriting — Relief SingleLine CAD
/// (SIL OFL 1.1). Unlike the filled `FONT`, its glyphs are open single-line paths, so tracing
/// one gives a pen *stroke*, not an outline.
static STROKE_FONT: &[u8] = include_bytes!("../assets/relief-singleline.ttf");

/// Glowing cyan for splat-text (sub-1 so HDR bloom doesn't blow it into a blob).
pub const TEXT_RGB: [f32; 3] = [0.40 * 0.8, 0.85 * 0.8, 1.0 * 0.8];

/// 3DGS degree-0 encode: rendered colour ≈ 0.5 + 0.2820948*dc, so invert for a target sRGB.
fn dc(c: f32) -> f32 {
    (c - 0.5) / 0.282_094_79
}

/// Rasterize `s` to flat gaussians on z=0 (centered at origin), scaled so the block spans
/// `world_width`. One small gaussian per sampled glyph-coverage pixel; opacity = coverage.
pub fn build_text_gaussians(s: &str, rgb: [f32; 3], world_width: f32, stride: usize, splat: f32) -> Vec<Gaussian3d> {
    let font = FontRef::try_from_slice(FONT).expect("font.ttf");
    let px = 64.0_f32;
    let sf = font.as_scaled(PxScale::from(px));
    let line_h = sf.height() + sf.line_gap();

    // layout: pen positions (baseline) per glyph, with kerning + newlines
    let mut placed: Vec<(f32, f32, char)> = Vec::new();
    let (mut pen_x, mut pen_y, mut max_x) = (0.0_f32, sf.ascent(), 0.0_f32);
    let mut prev: Option<char> = None;
    for ch in s.chars() {
        if ch == '\n' {
            pen_x = 0.0;
            pen_y += line_h;
            prev = None;
            continue;
        }
        if let Some(p) = prev {
            pen_x += sf.kern(font.glyph_id(p), font.glyph_id(ch));
        }
        placed.push((pen_x, pen_y, ch));
        pen_x += sf.h_advance(font.glyph_id(ch));
        max_x = max_x.max(pen_x);
        prev = Some(ch);
    }
    let block_h = pen_y + sf.descent().abs();
    let scale = world_width / max_x.max(1.0);
    let (cx, cy) = (max_x * 0.5, block_h * 0.5);

    let mut sh = SphericalHarmonicCoefficients::default();
    sh.set(0, dc(rgb[0]));
    sh.set(1, dc(rgb[1]));
    sh.set(2, dc(rgb[2]));

    let mut out: Vec<Gaussian3d> = Vec::new();
    let mut i: u32 = 0;
    for (gx0, gy0, ch) in &placed {
        let glyph = font.glyph_id(*ch).with_scale_and_position(px, point(*gx0, *gy0));
        let Some(o) = font.outline_glyph(glyph) else { continue }; // spaces -> no outline
        let bb = o.px_bounds();
        let (w, h) = (bb.width().ceil() as usize + 1, bb.height().ceil() as usize + 1);
        let mut cov = vec![0f32; w * h];
        o.draw(|dx, dy, c| {
            let (x, y) = (dx as usize, dy as usize);
            if x < w && y < h {
                cov[y * w + x] = c;
            }
        });
        for yy in (0..h).step_by(stride) {
            for xx in (0..w).step_by(stride) {
                let c = cov[yy * w + xx];
                if c < 0.35 {
                    continue; // coverage threshold → clean edges
                }
                // cheap deterministic jitter inside the cell (no rng dep)
                let j = |k: u32| ((k.wrapping_mul(2_654_435_761) >> 8) & 0xff) as f32 / 255.0 - 0.5;
                let gpx = bb.min.x + xx as f32 + j(i) * stride as f32;
                let gpy = bb.min.y + yy as f32 + j(i ^ 0x9e37) * stride as f32;
                i = i.wrapping_add(1);
                let wx = (gpx - cx) * scale;
                let wy = (gpy - cy) * scale; // Y-DOWN; cloud_base_rotation flips it upright
                out.push(Gaussian3d {
                    position_visibility: [wx, wy, 0.0, 1.0].into(),
                    spherical_harmonic: sh,
                    rotation: [0.0, 0.0, 0.0, 1.0].into(),
                    scale_opacity: [splat, splat, splat, c].into(),
                });
            }
        }
    }
    out
}

/// Flatten one glyph `OutlineCurve` into points (font units) ~`step_fu` apart, pushing
/// `(x, y, seg_len)` where `seg_len` is the distance from the previous point of THIS curve
/// (0 for its first point, so jumps between contours don't count as drawn pen length).
fn sample_curve(c: &OutlineCurve, step_fu: f32, out: &mut Vec<(f32, f32, f32)>) {
    let pts: Vec<(f32, f32)> = match c {
        OutlineCurve::Line(a, b) => {
            let n = ((b.x - a.x).hypot(b.y - a.y) / step_fu).ceil().max(1.0) as usize;
            (0..=n)
                .map(|i| {
                    let t = i as f32 / n as f32;
                    (a.x + (b.x - a.x) * t, a.y + (b.y - a.y) * t)
                })
                .collect()
        }
        OutlineCurve::Quad(a, c1, b) => {
            let approx = (c1.x - a.x).hypot(c1.y - a.y) + (b.x - c1.x).hypot(b.y - c1.y);
            let n = (approx / step_fu).ceil().max(2.0) as usize;
            (0..=n)
                .map(|i| {
                    let t = i as f32 / n as f32;
                    let u = 1.0 - t;
                    (u * u * a.x + 2.0 * u * t * c1.x + t * t * b.x, u * u * a.y + 2.0 * u * t * c1.y + t * t * b.y)
                })
                .collect()
        }
        OutlineCurve::Cubic(a, c1, c2, b) => {
            let approx = (c1.x - a.x).hypot(c1.y - a.y) + (c2.x - c1.x).hypot(c2.y - c1.y) + (b.x - c2.x).hypot(b.y - c2.y);
            let n = (approx / step_fu).ceil().max(3.0) as usize;
            (0..=n)
                .map(|i| {
                    let t = i as f32 / n as f32;
                    let u = 1.0 - t;
                    (
                        u * u * u * a.x + 3.0 * u * u * t * c1.x + 3.0 * u * t * t * c2.x + t * t * t * b.x,
                        u * u * u * a.y + 3.0 * u * u * t * c1.y + 3.0 * u * t * t * c2.y + t * t * t * b.y,
                    )
                })
                .collect()
        }
    };
    for (i, &(x, y)) in pts.iter().enumerate() {
        let seg = if i == 0 { 0.0 } else { (x - pts[i - 1].0).hypot(y - pts[i - 1].1) };
        out.push((x, y, seg));
    }
}

/// (impl) Sample gaussians ALONG each glyph's outline of `font_bytes` in pen (contour) order,
/// with each one's normalized cumulative pen-distance baked into the **visibility** channel
/// (`w`). Shader transition mode 7 reads that as its per-particle phase, so the stroke reveals
/// in writing order. Flat at z=0 (unlike baking into z) so the morph target is unaffected. A
/// FILLED font traces letter *outlines* (neon draw-on); a SINGLE-STROKE font traces centerlines
/// (true handwriting).
fn pen_gaussians(font_bytes: &[u8], s: &str, rgb: [f32; 3], world_width: f32, step_px: f32, splat: f32) -> Vec<Gaussian3d> {
    let font = FontRef::try_from_slice(font_bytes).expect("font");
    let px = 64.0_f32;
    let sf = font.as_scaled(PxScale::from(px));
    let upm = font.units_per_em().unwrap_or(1000.0);
    let s_f = px / upm; // font units → px
    let step_fu = (step_px / s_f).max(1.0);

    // layout: same pen walk as build_text_gaussians (kerning + newlines)
    let line_h = sf.height() + sf.line_gap();
    let mut placed: Vec<(f32, f32, char)> = Vec::new();
    let (mut pen_x, mut pen_y, mut max_x) = (0.0_f32, sf.ascent(), 0.0_f32);
    let mut prev: Option<char> = None;
    for ch in s.chars() {
        if ch == '\n' {
            pen_x = 0.0;
            pen_y += line_h;
            prev = None;
            continue;
        }
        if let Some(p) = prev {
            pen_x += sf.kern(font.glyph_id(p), font.glyph_id(ch));
        }
        placed.push((pen_x, pen_y, ch));
        pen_x += sf.h_advance(font.glyph_id(ch));
        max_x = max_x.max(pen_x);
        prev = Some(ch);
    }
    let block_h = pen_y + sf.descent().abs();
    let scale = world_width / max_x.max(1.0);
    let (cx, cy) = (max_x * 0.5, block_h * 0.5);

    let mut sh = SphericalHarmonicCoefficients::default();
    sh.set(0, dc(rgb[0]));
    sh.set(1, dc(rgb[1]));
    sh.set(2, dc(rgb[2]));

    // walk outlines left→right glyph by glyph, accumulating drawn pen length
    let mut samples: Vec<(f32, f32, f32)> = Vec::new();
    let mut acc = 0.0_f32;
    for (gx0, gy0, ch) in &placed {
        let Some(outline) = font.outline(font.glyph_id(*ch)) else { continue }; // spaces → none
        for curve in &outline.curves {
            let mut cpts: Vec<(f32, f32, f32)> = Vec::new();
            sample_curve(curve, step_fu, &mut cpts);
            for (fx, fy, seg) in cpts {
                acc += seg;
                samples.push((gx0 + fx * s_f, gy0 - fy * s_f, acc)); // font y-up → screen y-down
            }
        }
    }
    let total = acc.max(1e-6);

    samples
        .into_iter()
        .map(|(sx, sy, len)| {
            let wx = (sx - cx) * scale;
            let wy = (sy - cy) * scale;
            let phase = (len / total).clamp(0.0, 1.0); // → visibility (w); shader mode 7 reads it
            Gaussian3d {
                position_visibility: [wx, wy, 0.0, phase].into(),
                spherical_harmonic: sh,
                rotation: [0.0, 0.0, 0.0, 1.0].into(),
                scale_opacity: [splat, splat, splat, 1.0].into(),
            }
        })
        .collect()
}

/// Outline draw-on: trace the FILLED bundled font's glyph outlines in pen order — a glowing
/// neon outline that writes itself on. (Pairs with `Transition::Outline` / `~outline`.)
pub fn build_text_outline_gaussians(s: &str, rgb: [f32; 3], world_width: f32, step_px: f32, splat: f32) -> Vec<Gaussian3d> {
    pen_gaussians(FONT, s, rgb, world_width, step_px, splat)
}

/// Collects a glyph's contours as polylines in font units, flattening curves but **ignoring
/// `close()`** — so a single-stroke font's open centerline path stays open (closing it would
/// turn an `E`/`S`/`F` stroke into a box/8, which is what ab_glyph does).
#[derive(Default)]
struct OpenContours {
    contours: Vec<Vec<(f32, f32)>>,
    cur: Vec<(f32, f32)>,
}
impl OpenContours {
    fn flush(&mut self) {
        if self.cur.len() > 1 {
            self.contours.push(std::mem::take(&mut self.cur));
        } else {
            self.cur.clear();
        }
    }
}
impl ttf_parser::OutlineBuilder for OpenContours {
    fn move_to(&mut self, x: f32, y: f32) {
        self.flush();
        self.cur.push((x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.cur.push((x, y));
    }
    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let &(x0, y0) = self.cur.last().unwrap_or(&(x, y));
        for i in 1..=8 {
            let t = i as f32 / 8.0;
            let u = 1.0 - t;
            self.cur.push((u * u * x0 + 2.0 * u * t * x1 + t * t * x, u * u * y0 + 2.0 * u * t * y1 + t * t * y));
        }
    }
    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let &(x0, y0) = self.cur.last().unwrap_or(&(x, y));
        for i in 1..=10 {
            let t = i as f32 / 10.0;
            let u = 1.0 - t;
            self.cur.push((
                u * u * u * x0 + 3.0 * u * u * t * x1 + 3.0 * u * t * t * x2 + t * t * t * x,
                u * u * u * y0 + 3.0 * u * u * t * y1 + 3.0 * u * t * t * y2 + t * t * t * y,
            ));
        }
    }
    fn close(&mut self) {} // keep OPEN — single-stroke centerline, not a closed loop
}

/// True pen-write: trace the SINGLE-STROKE font's centerline in pen order — actual handwriting.
/// Uses `ttf-parser` (not ab_glyph) so contours stay OPEN; cumulative pen-distance is baked into
/// the visibility channel for shader mode 7. Single line only. (Pairs with `~pen-write`.)
pub fn build_text_penwrite_gaussians(s: &str, rgb: [f32; 3], world_width: f32, step_px: f32, splat: f32) -> Vec<Gaussian3d> {
    let Ok(face) = ttf_parser::Face::parse(STROKE_FONT, 0) else { return Vec::new() };
    let upm = face.units_per_em() as f32;
    let (asc, desc) = (face.ascender() as f32, face.descender() as f32);
    let step_fu = (step_px * upm / 64.0).max(1.0); // sample spacing in font units

    // layout (font units, y-down to match build_text_gaussians + cloud_base_rotation flip)
    let mut placed: Vec<(f32, char)> = Vec::new();
    let (mut pen_x, mut max_x) = (0.0_f32, 0.0_f32);
    for ch in s.chars().filter(|&c| c != '\n') {
        placed.push((pen_x, ch));
        let adv = face.glyph_index(ch).and_then(|g| face.glyph_hor_advance(g)).unwrap_or((upm * 0.5) as u16) as f32;
        pen_x += adv;
        max_x = max_x.max(pen_x);
    }
    let baseline = asc;
    let block_h = (asc - desc).max(1.0);
    let scale = world_width / max_x.max(1.0);
    let (cx, cy) = (max_x * 0.5, block_h * 0.5);

    let mut sh = SphericalHarmonicCoefficients::default();
    sh.set(0, dc(rgb[0]));
    sh.set(1, dc(rgb[1]));
    sh.set(2, dc(rgb[2]));

    // walk OPEN contours, resample each by step_fu, accumulate global pen length for the phase
    let mut samples: Vec<(f32, f32, f32)> = Vec::new();
    let mut acc = 0.0_f32;
    for (px0, ch) in &placed {
        let Some(gid) = face.glyph_index(*ch) else { continue }; // spaces → no glyph
        let mut ob = OpenContours::default();
        face.outline_glyph(gid, &mut ob);
        ob.flush();
        for contour in &ob.contours {
            let Some(&(mut ax, mut ay)) = contour.first() else { continue };
            samples.push((px0 + ax, baseline - ay, acc)); // contour start: no jump counted
            for &(bx, by) in &contour[1..] {
                let seglen = (bx - ax).hypot(by - ay);
                let n = (seglen / step_fu).ceil().max(1.0) as usize;
                for i in 1..=n {
                    let t = i as f32 / n as f32;
                    acc += seglen / n as f32;
                    samples.push((px0 + ax + (bx - ax) * t, baseline - (ay + (by - ay) * t), acc));
                }
                ax = bx;
                ay = by;
            }
        }
    }
    let total = acc.max(1e-6);

    samples
        .into_iter()
        .map(|(fx, fy, len)| {
            let phase = (len / total).clamp(0.0, 1.0);
            Gaussian3d {
                position_visibility: [(fx - cx) * scale, (fy - cy) * scale, 0.0, phase].into(),
                spherical_harmonic: sh,
                rotation: [0.0, 0.0, 0.0, 1.0].into(),
                scale_opacity: [splat, splat, splat, 1.0].into(),
            }
        })
        .collect()
}
