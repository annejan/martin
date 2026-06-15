//! Build-time logic: once every referenced splat has loaded, resolve each part's modes, build its
//! shape (resampled to the fixed count) + intro ball + source/out clouds, spawn the single
//! interpolate entity, seed the framing camera, and (for `glb:` parts) spawn the dissolve mesh.

use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy_gaussian_splatting::morph::interpolate::GaussianInterpolate;
use bevy_gaussian_splatting::sort::SortMode;
use bevy_gaussian_splatting::{
    CloudSettings, Gaussian3d, PlanarGaussian3d, PlanarGaussian3dHandle, RasterizeMode,
};

use super::model::{BuiltShot, SeqState, Sequence, shot_starts};
use super::parse::{global_raster, parse_euler_deg};
use crate::camera::{DEFAULT_PITCH, FRONT_YAW, OrbitCam};
use crate::morph::{ball_of, resample_morton};
use crate::scene::content::{PartContent, part_gaussians};
use crate::scene::effects::{BALL_SHELL, Deform, Transition, source_cloud};
use crate::scene::gl_dissolve::spawn_gl_dissolve;
use crate::scene::{AssetRoot, NORMALIZE_EXTENT, cloud_base_rotation};
use crate::text::{TEXT_RGB, build_text_outline_gaussians, build_text_penwrite_gaussians};

/// Once every referenced splat has loaded, build each part's shape (resampled to the fixed
/// count) + the intro ball, spawn the single interpolate entity, and frame the union once.
pub(crate) fn build_sequence(
    mut commands: Commands,
    mut assets: ResMut<Assets<PlanarGaussian3d>>,
    seq: Option<Res<Sequence>>,
    state: Option<ResMut<SeqState>>,
    root: Res<AssetRoot>,
    asset_server: Res<AssetServer>,
    mut cam: Query<&mut OrbitCam>,
) {
    let (Some(seq), Some(mut state)) = (seq, state) else {
        return;
    };
    if state.built || seq.parts.is_empty() {
        return;
    }
    if state.loads.iter().any(|h| assets.get(h).is_none()) {
        return; // wait for every referenced splat
    }

    // resolve each part's transition first (explicit ~name > MARTIN_TRANSITION > Ball for part
    // 0 / Morph after) — needed before building gaussians so a PenWrite text part is built as a
    // stroked outline (pen order baked into visibility) instead of filled coverage.
    let global_tr = std::env::var("MARTIN_TRANSITION")
        .ok()
        .and_then(|s| Transition::parse(&s));
    // persistent per-part deform: explicit `^name` > MARTIN_DEFORM > none. Runs while the part is
    // held (a waving wall of text etc.), independent of the arrival transition above.
    let global_deform = std::env::var("MARTIN_DEFORM")
        .ok()
        .and_then(|s| Deform::parse(&s));
    let deforms: Vec<Option<Deform>> = seq
        .parts
        .iter()
        .map(|p| p.deform.or(global_deform))
        .collect();
    let global_raster = global_raster();
    let rasters: Vec<RasterizeMode> = seq
        .parts
        .iter()
        .map(|p| p.raster.unwrap_or(global_raster))
        .collect();
    let transitions: Vec<Transition> = seq
        .parts
        .iter()
        .enumerate()
        .map(|(idx, part)| {
            let tr = part.transition.or(global_tr).unwrap_or(if idx == 0 {
                Transition::Ball
            } else {
                Transition::Morph
            });
            // part 0 has nothing to morph from — fall back to a ball assemble.
            if idx == 0 && matches!(tr, Transition::Morph | Transition::Swarm) {
                Transition::Ball
            } else {
                tr
            }
        })
        .collect();

    // Absolute start time (s) of each shot — the cue timeline (anchors, else laid end-to-end).
    let starts = shot_starts(&seq.parts);

    // read every part's gaussians once, so count==0 can mean "size N to the largest part"
    // (every part is then resampled to that single N — required by the shared morph output).
    // pen-write strokes are thin: MARTIN_PW_SPLAT (gaussian size) / MARTIN_PW_STEP (sample
    // spacing) tune stroke weight — a fat splat blooms the strokes into filled blobs.
    let pw_step = crate::envvar::or("MARTIN_PW_STEP", 0.5_f32);
    let pw_splat = crate::envvar::or("MARTIN_PW_SPLAT", 0.006_f32);
    let mut raws: Vec<Vec<Gaussian3d>> = seq
        .parts
        .iter()
        .zip(&transitions)
        .map(|(part, &tr)| match (&part.content, tr) {
            (PartContent::Text(s), Transition::Outline) => {
                build_text_outline_gaussians(s, TEXT_RGB, 3.0, 0.7, 0.012)
            }
            (PartContent::Text(s), Transition::PenWrite) => {
                build_text_penwrite_gaussians(s, TEXT_RGB, 3.0, pw_step, pw_splat)
            }
            _ => part_gaussians(&part.content, &state, &assets, &root.0),
        })
        .collect();
    // `cluster:N` → replicate a part into N scattered, randomly-rotated copies (a "serving", e.g. a
    // pile of bitterballen) BEFORE normalize, so the whole pile frames as one. Downsample per copy
    // to keep the total near the morph budget.
    // a shot's cluster needs a concrete per-copy budget; when `budget==0` (auto-size to the largest
    // shot) fall back to 200k here rather than 0 — so this default is intentionally NOT `seq.budget`.
    let cluster_total = crate::envvar::or("MARTIN_MORPH_COUNT", 200_000usize);
    for (raw, part) in raws.iter_mut().zip(&seq.parts) {
        if let Some(copies) = part.cluster {
            let per = (cluster_total / copies.max(1)).max(2_000);
            let one = resample_morton(std::mem::take(raw), per);
            *raw = crate::morph::cluster_of(&one, copies);
        }
    }
    // Normalize each part to a common "normal" size (MARTIN_NORMALIZE=0 to disable). Sources
    // vary wildly — a COLMAP scene spans hundreds of units, a TRELLIS object ~1 — so without
    // this they'd frame inconsistently and morph badly. We log the raw extent first.
    let normalize = std::env::var("MARTIN_NORMALIZE")
        .map(|v| v != "0")
        .unwrap_or(true);
    let mut scene_norm = (Vec3::ZERO, 1.0); // part 0's (center, scale) — to transform camera poses
    for (i, (raw, part)) in raws.iter_mut().zip(&seq.parts).enumerate() {
        let label = part.content.label();
        info!(
            "part {label}: raw extent {:.2} units ({} gaussians)",
            crate::morph::extent_of(raw),
            raw.len()
        );
        // a glb: part is a placeholder here (sample_gl_mesh fills + normalizes it from the loaded
        // mesh later) — don't normalize the placeholder (its zero extent would blow up the scale).
        // a `cluster:` part is already frame-sized by cluster_of (the whole serving ≈ NORMALIZE_EXTENT),
        // so don't re-normalize it (that would re-fit on the 90th-percentile and shrink the pile).
        if normalize && !matches!(part.content, PartContent::GlMesh(_)) && part.cluster.is_none() {
            let norm = crate::morph::normalize_to(raw, NORMALIZE_EXTENT);
            if i == 0 {
                scene_norm = norm;
            }
        }
    }
    let n = if seq.budget > 0 {
        seq.budget
    } else {
        raws.iter().map(Vec::len).max().unwrap_or(0).max(1)
    };

    let mut union_lo = Vec3::splat(f32::MAX);
    let mut union_hi = Vec3::splat(f32::MIN);
    for g in raws.iter().flatten() {
        let p = Vec3::from_array(g.position_visibility.position);
        union_lo = union_lo.min(p);
        union_hi = union_hi.max(p);
    }

    // Framing radius of the *content*: when normalized, every part is ~NORMALIZE_EXTENT across
    // centred on its centroid, so frame from that — robust to the floaters that still inflate
    // the raw union AABB and would otherwise shrink the scene to a distant dot. Raw mode (no
    // normalize) frames the union box instead. This radius also sizes each transition source.
    let (frame_center, content_radius, frame_factor) = if normalize {
        (Vec3::ZERO, NORMALIZE_EXTENT * 0.5, 2.5)
    } else {
        let c = (union_lo + union_hi) * 0.5;
        (c, ((union_hi - union_lo) * 0.5).length().max(0.1), 1.7)
    };

    // Each part is resampled to the shared count N, then gets the *source* cloud it morphs in
    // FROM, chosen by its transition (`~name` per part > MARTIN_TRANSITION default > Ball for
    // part 0 / Morph after). `Morph` has no source — it flows from the previous part's shape
    // (with the ball-pulse bulge); the others build a source from the part's own shape.
    let mut shapes = Vec::new();
    let mut sources: Vec<Option<Handle<PlanarGaussian3d>>> = Vec::new();
    let mut out_clouds: Vec<Option<Handle<PlanarGaussian3d>>> = Vec::new();
    // MARTIN_PAIR=match (or `pair=match` in [settings]): instead of index-rank Morton pairing — which
    // pinches DISSIMILAR scenes through a centre "ball" — reorder each Morph part so splat k pairs with
    // a nearby, similar-colour splat in the PREVIOUS part (greedy bijective nearest match, cost =
    // pos² + color_w·colour²). Short colour-matched moves → a coherent ghostly morph (grass→trees,
    // tower→tower), no centre-collapse. MARTIN_PAIR_COLOR weights colour vs position (default 0.5).
    let pair_match = std::env::var("MARTIN_PAIR")
        .map(|v| v.eq_ignore_ascii_case("match"))
        .unwrap_or(false);
    let pair_color_w = crate::envvar::or("MARTIN_PAIR_COLOR", 0.5_f32);
    let mut prev_shaped: Option<Vec<Gaussian3d>> = None;
    for (idx, raw) in raws.into_iter().enumerate() {
        // ROBUSTNESS: a part that produced 0 gaussians (an unsupported/broken asset) must NOT reach
        // the morph shader — an empty cloud is a wgpu validation error that crashes the whole render.
        // Degrade it to a transparent placeholder so the show plays on (the part is just invisible).
        let raw = if raw.is_empty() {
            warn!(
                "part {}: 0 gaussians — substituting a transparent placeholder (asset failed to load?)",
                seq.parts[idx].content.label()
            );
            crate::mesh::transparent_placeholder(256)
        } else {
            raw
        };
        let mut shaped = resample_morton(raw, n);
        // bake the part's own `rot:` into its shape (so sources/out clouds inherit it, and the morph
        // between differently-oriented parts reorients smoothly). glb parts rotate in sample_gl_mesh.
        if let Some(q) = seq.parts[idx].rot {
            if !matches!(seq.parts[idx].content, PartContent::GlMesh(_)) {
                crate::morph::rotate_gaussians(&mut shaped, q);
            }
        }
        let tr = transitions[idx];
        let r = content_radius;
        // if the PREVIOUS part DEPARTS (washes/disperses away), there's no shape to flow from →
        // a Morph/Swarm part must assemble fresh from a ball instead.
        let prev_departs = idx > 0 && seq.parts[idx - 1].out.is_some();
        // pair=match: when this Morph part flows DIRECTLY from the previous shape (no departure, no
        // source cloud), reorder it so each splat slides to the nearest same-colour splat of the prev
        // part — minimal travel, coherent morph. Only the Morph/Swarm-flow case has a prev shape to
        // pair against; sourced transitions (ball/wash/etc.) assemble from their own cloud regardless.
        if pair_match && matches!(tr, Transition::Morph | Transition::Swarm) && !prev_departs {
            if let Some(prev) = &prev_shaped {
                shaped = crate::morph::match_reorder(prev, shaped, pair_color_w);
            }
        }
        let src: Option<Vec<Gaussian3d>> = match tr {
            // Morph/Swarm flow from the PREVIOUS part's shape (no source) — unless the previous part
            // departed (washed away), in which case there's nothing to flow from → assemble fresh.
            Transition::Morph | Transition::Swarm if prev_departs => {
                Some(ball_of(&shaped, r * BALL_SHELL))
            }
            Transition::Morph | Transition::Swarm => None,
            other => source_cloud(other, &shaped, r),
        };
        // `out:` departure target cloud (faded + displaced) — the part morphs to this as it leaves.
        let out = seq.parts[idx].out.map(|d| d.out_cloud(&shaped, r));
        sources.push(src.map(|s| assets.add(PlanarGaussian3d::from(s))));
        out_clouds.push(out.map(|o| assets.add(PlanarGaussian3d::from(o))));
        // keep this part's shape so the NEXT part can pair-match against it (only needed for pair=match;
        // the clone is gated to avoid copying a big cloud on every render otherwise).
        if pair_match {
            prev_shaped = Some(shaped.clone());
        }
        shapes.push(assets.add(PlanarGaussian3d::from(shaped)));
    }
    let intro0 = sources[0]
        .clone()
        .expect("part 0 always builds a source cloud");

    // MARTIN_ROT="rx,ry,rz" (euler degrees) orients the cloud — e.g. to stand a COLMAP scene
    // upright for a "normal" POV. Default = cloud_base_rotation (flip-X, right for portrait
    // splats; gives scenes their abstract sideways look).
    let entity_rot = std::env::var("MARTIN_ROT")
        .ok()
        .and_then(|s| parse_euler_deg(&s))
        .unwrap_or_else(cloud_base_rotation);
    // MARTIN_REEL_POS="x,y,z" translates the whole morph timeline off the world origin (default 0,0,0).
    // The reel normally sits at the origin; this lets you place the morphing subject relative to
    // `[stage]` props (which carry their own `@x,y,z`) — e.g. float the morph above a placed cityscape.
    let reel_pos = std::env::var("MARTIN_REEL_POS")
        .ok()
        .and_then(|s| {
            let mut it = s.split(',').map(|c| c.trim().parse::<f32>().ok());
            Some(Vec3::new(it.next()??, it.next()??, it.next()??))
        })
        .unwrap_or(Vec3::ZERO);

    let entity = commands
        .spawn((
            GaussianInterpolate::<Gaussian3d> {
                lhs: PlanarGaussian3dHandle(intro0.clone()),
                rhs: PlanarGaussian3dHandle(shapes[0].clone()),
            },
            CloudSettings {
                sort_mode: SortMode::Radix,
                time: 0.0, // start as the ball; shot_director morphs it in
                time_start: 0.0,
                time_stop: 1.0,
                bulge: 0.0,
                ..default()
            },
            Transform::from_rotation(entity_rot).with_translation(reel_pos),
        ))
        .id();

    // frame the union once (camera never pops between parts); apply the same rotation to the
    // centre so the camera looks at the post-transform world centre.
    // Seed the free-orbit camera. MARTIN_ZOOM scales distance (>1 = closer); MARTIN_YAW /
    // MARTIN_PITCH (radians) seed the orbit angle so you can bake a found viewpoint into a
    // render (and freely orbit live from there).
    use crate::envvar::or as env;
    let zoom = env("MARTIN_ZOOM", 1.0_f32);
    let zoom = if zoom > 0.0 { zoom } else { 1.0 }; // a non-positive zoom is meaningless → default
    let center = entity_rot * frame_center;
    let (mut yaw, mut pitch, mut dist) = (
        env("MARTIN_YAW", FRONT_YAW),
        env("MARTIN_PITCH", DEFAULT_PITCH),
        content_radius * frame_factor / zoom,
    );
    // MARTIN_CAMERAS=<cameras.json>: park the camera at a real capture pose (the only viewpoint
    // a raw 360° scene renders coherently). Transform the chosen capture position through the
    // SAME normalize (part 0) + cloud rotation as the gaussians, then read off yaw/pitch/dist
    // around the framed centre. MARTIN_CAM_INDEX picks which shot (default 0).
    if let Ok(cpath) = std::env::var("MARTIN_CAMERAS") {
        let positions = super::parse::load_camera_positions(&cpath);
        if positions.is_empty() {
            warn!("MARTIN_CAMERAS: no camera positions in {cpath}");
        } else {
            let idx = std::env::var("MARTIN_CAM_INDEX")
                .ok()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(0)
                .min(positions.len() - 1);
            let (c0, s0) = scene_norm;
            let dir = entity_rot * ((positions[idx] - c0) * s0) - center;
            let len = dir.length().max(1e-4);
            yaw = dir.z.atan2(dir.x);
            pitch = (dir.y / len).asin();
            dist = len / zoom;
            info!(
                "camera: capture pose {idx}/{} from {cpath}",
                positions.len()
            );
        }
    }
    for mut c in &mut cam {
        c.target = center;
        c.dist = dist;
        c.yaw = yaw;
        c.pitch = pitch;
        c.framed = true;
    }

    // `glb:` parts: render the real mesh AND sample its gaussians from that same mesh (filled by
    // sample_gl_mesh) so the mesh can dissolve into its own splats. Shares the cloud's base frame.
    for (idx, part) in seq.parts.iter().enumerate() {
        if let PartContent::GlMesh(name) = &part.content {
            spawn_gl_dissolve(
                &mut commands,
                &asset_server,
                name,
                idx,
                entity_rot,
                part.rot.unwrap_or(Quat::IDENTITY),
                shapes[idx].clone(),
                n,
            );
        }
    }

    // Collapse the index-parallel build outputs into one `BuiltShot` per shot — the only per-shot
    // data the director reads. Each `Vec` is consumed in lock-step (all are `seq.parts.len()` long).
    let mut shots = Vec::with_capacity(seq.parts.len());
    let mut shapes = shapes.into_iter();
    let mut sources = sources.into_iter();
    let mut out_clouds = out_clouds.into_iter();
    for ((((shot, transition), deform), raster), start) in seq
        .parts
        .iter()
        .zip(transitions)
        .zip(deforms)
        .zip(rasters)
        .zip(starts)
    {
        shots.push(BuiltShot {
            shape: shapes.next().expect("one shape per shot"),
            origin: sources.next().expect("one source per shot"),
            out_cloud: out_clouds.next().expect("one out-cloud per shot"),
            transition,
            deform,
            deform_amp: shot.deform_amp,
            flash: shot.flash,
            beat: shot.beat,
            raster,
            start,
            morph: shot.morph,
            bulge: shot.bulge,
            out: shot.out,
            is_gl_mesh: matches!(shot.content, PartContent::GlMesh(_)),
        });
    }
    let built_n = shots.len();
    state.shots = shots;
    state.entity = Some(entity);
    state.built = true;
    info!("sequence built: {built_n} shots × {n} gaussians");
}

/// Add `NoFrustumCulling` to the sequence entity once its Aabb exists, so morph/ball
/// particles that briefly leave the framed view don't pop out.
#[allow(clippy::type_complexity)] // a Bevy query filter tuple — verbose by nature
pub(crate) fn seq_no_cull(
    mut commands: Commands,
    state: Option<Res<SeqState>>,
    q: Query<
        (),
        (
            With<GaussianInterpolate<Gaussian3d>>,
            With<Aabb>,
            Without<NoFrustumCulling>,
        ),
    >,
) {
    let Some(state) = state else { return };
    let Some(e) = state.entity else { return };
    if q.get(e).is_ok() {
        commands.entity(e).insert(NoFrustumCulling);
    }
}
