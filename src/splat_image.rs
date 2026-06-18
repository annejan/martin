//! Splat-image: sample an image's (PNG, or a rasterized SVG) opaque pixels into flat (z=0) coloured
//! gaussians, so a logo or any vector/raster art is just another morph source — it can ball-assemble,
//! sparkle in, hold, and morph into the next part exactly like splat-text. Built **Y-DOWN** so the
//! entity's `cloud_base_rotation` flips it upright, matching text and the Y-down `.ply` splats.

use bevy_gaussian_splatting::{Gaussian3d, SphericalHarmonicCoefficients};

/// 3DGS degree-0 encode: rendered colour ≈ 0.5 + 0.2820948·dc, so invert for a target linear.
fn dc(c: f32) -> f32 {
    (c - 0.5) / 0.282_094_79
}

/// Sample the opaque pixels of a PNG/JPEG (every `stride`-th pixel) into colored gaussians. Reads
/// `MARTIN_IMG_STRIDE` and `MARTIN_IMG_SPLAT` from the env.
pub fn build_image_gaussians(
    png: &[u8],
    world_width: f32,
    alpha_thresh: f32,
    gain: f32,
) -> Vec<Gaussian3d> {
    let stride = crate::envvar::or("MARTIN_IMG_STRIDE", 2);
    let splat = crate::envvar::or("MARTIN_IMG_SPLAT", 0.012);
    let img = match image::load_from_memory(png) {
        Ok(i) => i.to_rgba8(),
        Err(_) => return Vec::new(),
    };
    sample_rgba(&img, world_width, stride, splat, alpha_thresh, gain)
}

/// Rasterize an SVG to a raster and sample it into gaussians, exactly like a PNG logo — so any
/// vector art is a morph source. Reads `MARTIN_IMG_STRIDE`, `MARTIN_IMG_SPLAT`, and `MARTIN_SVG_PX`
/// from the env.
pub fn build_svg_gaussians(
    svg: &[u8],
    world_width: f32,
    alpha_thresh: f32,
    gain: f32,
) -> Vec<Gaussian3d> {
    let stride = crate::envvar::or("MARTIN_IMG_STRIDE", 2);
    let splat = crate::envvar::or("MARTIN_IMG_SPLAT", 0.012);
    let px = crate::envvar::or("MARTIN_SVG_PX", 512_u32);
    let Some(img) = rasterize_svg(svg, px) else {
        return Vec::new();
    };
    sample_rgba(&img, world_width, stride, splat, alpha_thresh, gain)
}

/// SVG bytes → an `RgbaImage` `px` wide (height preserves the aspect), straight (un-premultiplied)
/// alpha. `None` on a parse/alloc failure. Also used by the loader to show the logo SVG.
pub fn rasterize_svg(svg: &[u8], px: u32) -> Option<image::RgbaImage> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default()).ok()?;
    let size = tree.size();
    let px = px.clamp(16, 4096);
    let scale = px as f32 / size.width().max(1.0);
    let (w, h) = (px, (size.height() * scale).round().max(1.0) as u32);
    let mut pixmap = tiny_skia::Pixmap::new(w, h)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    // tiny-skia stores premultiplied RGBA; demultiply to straight colour for the sampler.
    let mut img = image::RgbaImage::new(w, h);
    for (dst, src) in img.pixels_mut().zip(pixmap.pixels()) {
        let c = src.demultiply();
        *dst = image::Rgba([c.red(), c.green(), c.blue(), c.alpha()]);
    }
    Some(img)
}

/// Sample the opaque pixels of an RGBA image (every `stride`-th pixel) into colored gaussians
/// spanning `world_width`, centred at the origin, flat on z=0. `alpha_thresh` drops near-transparent
/// pixels (clean edges); `gain` (<1) keeps bright logos from blooming into a blob. Deterministic
/// jitter (no rng) keeps record mode reproducible.
fn sample_rgba(
    img: &image::RgbaImage,
    world_width: f32,
    stride: usize,
    splat: f32,
    alpha_thresh: f32,
    gain: f32,
) -> Vec<Gaussian3d> {
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Vec::new();
    }
    let stride = stride.max(1);
    let scale = world_width / w as f32;
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);

    let mut out = Vec::new();
    let mut i: u32 = 0;
    for yy in (0..h).step_by(stride) {
        for xx in (0..w).step_by(stride) {
            let px = img.get_pixel(xx, yy).0;
            let a = px[3] as f32 / 255.0;
            if a < alpha_thresh {
                continue; // opaque pixels only → the logo shape, clean edges
            }
            let mut sh = SphericalHarmonicCoefficients::default();
            sh.set(0, dc(px[0] as f32 / 255.0 * gain));
            sh.set(1, dc(px[1] as f32 / 255.0 * gain));
            sh.set(2, dc(px[2] as f32 / 255.0 * gain));
            // cheap deterministic jitter inside the cell (mirrors text.rs; no rng dep)
            let j = |k: u32| ((k.wrapping_mul(2_654_435_761) >> 8) & 0xff) as f32 / 255.0 - 0.5;
            let gx = (xx as f32 + j(i) * stride as f32 - cx) * scale;
            let gy = (yy as f32 + j(i ^ 0x9e37) * stride as f32 - cy) * scale; // Y-DOWN
            i = i.wrapping_add(1);
            out.push(Gaussian3d {
                position_visibility: [gx, gy, 0.0, 1.0].into(),
                spherical_harmonic: sh,
                rotation: [0.0, 0.0, 0.0, 1.0].into(),
                scale_opacity: [splat, splat, splat, a].into(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_rgba_keeps_opaque_pixels_and_carries_alpha() {
        // a 2×2: three opaque pixels + one transparent → three gaussians (stride 1, thresh 0.5).
        let mut img = image::RgbaImage::new(2, 2);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));
        img.put_pixel(0, 1, image::Rgba([0, 0, 255, 255]));
        img.put_pixel(1, 1, image::Rgba([255, 255, 255, 0])); // transparent → dropped
        let g = sample_rgba(&img, 2.0, 1, 0.01, 0.5, 1.0);
        assert_eq!(g.len(), 3);
        assert!(g.iter().all(|x| x.scale_opacity.opacity == 1.0)); // alpha 255 → 1.0
        assert!(
            g.iter()
                .all(|x| { x.position_visibility.position.iter().all(|c| c.is_finite()) })
        );
        // an all-transparent image yields nothing.
        let blank = image::RgbaImage::new(4, 4);
        assert!(sample_rgba(&blank, 2.0, 1, 0.5, 0.5, 1.0).is_empty());
    }
}
