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

use super::model::{BuiltShot, SeqState, Sequence, Shot, shot_starts};
use super::parse::{global_raster, parse_euler_deg};
use crate::camera::OrbitCam;
use crate::morph::{ball_of, resample_morton};
use crate::scene::content::{PartContent, part_gaussians, sample_content_owned};
use crate::scene::effects::{BALL_SHELL, Deform, Entrance, source_cloud};
use crate::scene::gl_dissolve::spawn_gl_dissolve;
use crate::scene::{AssetRoot, NORMALIZE_EXTENT, cloud_base_rotation};

/// The union AABB `(min, max)` over every shot's raw gaussians — the extent raw (non-normalized) mode
/// frames the camera on.
fn union_bounds(raws: &[Vec<Gaussian3d>]) -> (Vec3, Vec3) {
    let mut lo = Vec3::splat(f32::MAX);
    let mut hi = Vec3::splat(f32::MIN);
    for g in raws.iter().flatten() {
        let p = Vec3::from_array(g.position_visibility.position);
        lo = lo.min(p);
        hi = hi.max(p);
    }
    (lo, hi)
}

/// Framing geometry `(center, content_radius, frame_factor)` for the union of all shots. When
/// normalized, every shot is ~`NORMALIZE_EXTENT` across centred on its centroid, so we frame from that
/// — robust to the floaters that inflate the raw union AABB and would otherwise shrink the scene to a
/// distant dot. Raw mode (no normalize) frames the union box instead. Pure — unit-tested.
fn frame_of(normalize: bool, union_lo: Vec3, union_hi: Vec3) -> (Vec3, f32, f32) {
    if normalize {
        (Vec3::ZERO, NORMALIZE_EXTENT * 0.5, 2.5)
    } else {
        let c = (union_lo + union_hi) * 0.5;
        (c, ((union_hi - union_lo) * 0.5).length().max(0.1), 1.7)
    }
}

/// Resolve a shot's arrival entrance: explicit `~name` > Ball for shot 0
/// / Morph after. Shot 0 has nothing to flow FROM, so Morph/Swarm/Cut degrade to a Ball assemble (the
/// caller logs the degrade). Pure — unit-tested.
fn arrival_entrance(idx: usize, explicit: Option<Entrance>, global: Option<Entrance>) -> Entrance {
    let tr = explicit.or(global).unwrap_or(if idx == 0 {
        Entrance::Ball
    } else {
        Entrance::Morph
    });
    // shot 0 has no previous shape to morph/swarm/cut from — degrade to a ball assemble.
    if idx == 0 && matches!(tr, Entrance::Morph | Entrance::Swarm | Entrance::Cut) {
        Entrance::Ball
    } else {
        tr
    }
}

/// Owned, Bevy-free inputs for the off-thread reel build (`build_cpu`). The main thread pre-extracts
/// the only asset-dependent content — `splat:` parts, copied out of `Assets` into `presampled` — so the
/// worker never touches the Bevy world. Everything else (`parts`, `root`) is owned/cloned.
struct BuildInputs {
    parts: Vec<Shot>,
    presampled: Vec<Option<Vec<Gaussian3d>>>, // Some only for `splat:` parts (extracted on main thread)
    root: std::path::PathBuf,
    budget: usize,
}

/// The pure-CPU result of `build_cpu` — the built shots (GPU handles still `None`, uploaded by
/// `ensure_window` on the main thread) + the framing geometry the camera seeding needs. No Bevy world.
struct BuildOutput {
    shots: Vec<BuiltShot>,
    frame_center: Vec3,
    content_radius: f32,
    frame_factor: f32,
    scene_norm: (Vec3, f32),
    n: usize,
    entity_rot: Quat,
}

/// Holds the in-flight off-thread reel build (live/windowed only); `None` when idle or done. Record/
/// shot/bench builds run inline (see `build_inline`) and never populate this.
#[derive(Resource, Default)]
pub(crate) struct SeqBuildTask(Option<bevy::tasks::Task<BuildOutput>>);

/// A deterministic one-shot capture (record/shot/bench) must have the reel fully built **before** the
/// first rendered/captured frame — so those modes build inline (same as before the off-thread split),
/// which also keeps the build a pure function of the inputs regardless of scheduling. Only the live
/// windowed run defers the build to a worker (the loader keeps animating instead of one frozen frame).
fn build_inline() -> bool {
    [
        "MARTIN_RECORD",
        "MARTIN_SHOT",
        "MARTIN_SHOTS",
        "MARTIN_BENCH",
    ]
    .iter()
    .any(|k| std::env::var_os(k).is_some())
}

/// The heavy, pure-CPU reel build: sample → flock → normalize → frame → `build_shots`. Takes owned
/// inputs (no Bevy world) so it can run on `AsyncComputeTaskPool` off the main thread. Deterministic —
/// a record bakes identically whether this finished on frame 0 or 3. The GPU upload + entity spawn +
/// camera seeding stay on the main thread (`finalize_build`).
fn build_cpu(inputs: BuildInputs) -> BuildOutput {
    let BuildInputs {
        parts,
        presampled,
        root,
        budget,
    } = inputs;

    // resolve each part's entrance first (explicit ~name > Ball for part
    // 0 / Morph after) — needed before building gaussians so a PenWrite text part is built as a
    // stroked outline (pen order baked into visibility) instead of filled coverage.
    // global default transition: none — each shot's per-part `~name` token is the knob.
    let global_tr: Option<Entrance> = None;
    // persistent per-part deform: explicit `^name` > MARTIN_DEFORM > none. Runs while the part is
    // held (a waving wall of text etc.), independent of the arrival entrance above.
    let global_deform = std::env::var("MARTIN_DEFORM")
        .ok()
        .and_then(|s| Deform::parse(&s));
    let deforms: Vec<Option<Deform>> = parts.iter().map(|p| p.deform.or(global_deform)).collect();
    let global_raster = global_raster();
    let rasters: Vec<RasterizeMode> = parts
        .iter()
        .map(|p| p.raster.unwrap_or(global_raster))
        .collect();
    // per-shot emissive glow blend: explicit `additive:0|1` > MARTIN_ADDITIVE. Resolved once here (not
    // per-frame) so the director just applies the concrete bool, same pattern as `raster` above.
    let global_additive = crate::envvar::or("MARTIN_ADDITIVE", 0) != 0;
    let additives: Vec<bool> = parts
        .iter()
        .map(|p| p.additive.unwrap_or(global_additive))
        .collect();
    let transitions: Vec<Entrance> = parts
        .iter()
        .enumerate()
        .map(|(idx, part)| {
            // shot 0 has nothing to morph/swarm/cut FROM — log the degrade (Morph falls back silently),
            // then `arrival_entrance` resolves it to a ball assemble.
            if idx == 0 {
                match part.entrance.or(global_tr) {
                    Some(Entrance::Cut) => {
                        warn!(
                            "seq: shot 0 can't ~cut (nothing to cut from) — assembling from a ball"
                        )
                    }
                    Some(Entrance::Swarm) => {
                        warn!("seq: shot 0 can't ~swarm (no prev shape) — assembling from a ball")
                    }
                    _ => {}
                }
            }
            arrival_entrance(idx, part.entrance, global_tr)
        })
        .collect();

    // Absolute start time (s) of each shot — the cue timeline (anchors, else laid end-to-end).
    let starts = shot_starts(&parts);

    // read every part's gaussians once, so count==0 can mean "size N to the largest part" (every part
    // is then resampled to that single N — required by the shared morph output). `sample_content_owned`
    // applies the `~outline`/`~pen-write` text-effect builders and consumes the pre-extracted `splat:`
    // clouds (no Bevy world here). PARALLEL: each part samples independently (mesh/text/svg rasterize is
    // the multi-second cost). `par_iter` is index-ordered → `collect` preserves part order (the morph
    // chain depends on it).
    use rayon::prelude::*;
    let mut raws: Vec<Vec<Gaussian3d>> = parts
        .par_iter()
        .zip(transitions.par_iter())
        .zip(presampled.into_par_iter())
        .map(|((part, &tr), pre)| {
            sample_content_owned(
                &part.content,
                Some(tr),
                pre,
                &root,
                part.disk,
                part.aniso,
                None, // reel meshes keep the 60k sample default (parts are paired at full count)
                part.font.as_deref(),
            )
        })
        .collect();
    // `cluster:N` → replicate a part into N scattered, randomly-rotated copies (a "serving", e.g. a
    // pile of bitterballen) BEFORE normalize, so the whole pile frames as one. Downsample per copy
    // to keep the total near the morph budget.
    // a shot's cluster needs a concrete per-copy budget; when `budget==0` (auto-size to the largest
    // shot) fall back to 200k here rather than 0 — so this default is intentionally NOT `budget`.
    let cluster_total = crate::scene::cap_count(
        "cluster",
        crate::envvar::or("MARTIN_MORPH_COUNT", 200_000usize),
    );
    for (raw, part) in raws.iter_mut().zip(&parts) {
        if let Some(copies) = part.flock {
            let per = (cluster_total / copies.max(1)).max(2_000);
            let one = resample_morton(std::mem::take(raw), per);
            *raw = crate::morph::cluster_of(&one, copies);
        }
    }
    // Normalize each part to a common "normal" size (always on — there is no disable knob). Sources
    // vary wildly — a COLMAP scene spans hundreds of units, a TRELLIS object ~1 — so without
    // this they'd frame inconsistently and morph badly. We log the raw extent first.
    let normalize = true; // always frame each part to a common extent (parts vary wildly in raw scale)
    let mut scene_norm = (Vec3::ZERO, 1.0); // part 0's (center, scale) — to transform camera poses
    for (i, (raw, part)) in raws.iter_mut().zip(&parts).enumerate() {
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
        if normalize && !matches!(part.content, PartContent::GlMesh(_)) && part.flock.is_none() {
            let norm = crate::morph::normalize_to(raw, NORMALIZE_EXTENT);
            if i == 0 {
                scene_norm = norm;
            }
        }
    }
    let n = if budget > 0 {
        budget
    } else {
        raws.iter().map(Vec::len).max().unwrap_or(0).max(1)
    };
    // MARTIN_COUNT_SCALE: global density multiplier (QUALITY low/potato dial it down) — scales even an
    // explicit `budget=` that MARTIN_MORPH_COUNT can't cap, so QUALITY thins any reel.
    let n = crate::scene::cap_count(
        "reel",
        ((n as f32 * crate::scene::count_scale()).round() as usize).max(256),
    );

    // Framing geometry of the *content* (see `frame_of`): normalized shots frame from the centroid
    // (robust to floaters that inflate the raw union AABB); raw mode frames the union box. The radius
    // also sizes each entrance source.
    let (union_lo, union_hi) = union_bounds(&raws);
    let (frame_center, content_radius, frame_factor) = frame_of(normalize, union_lo, union_hi);

    // MARTIN_ROT="rx,ry,rz" (euler degrees) orients the whole cloud; default = cloud_base_rotation
    // (Rx180 — .ply splats are Y-down). Computed BEFORE build_shots so `ground:` can seat parts in the
    // WORLD frame (this rotation flips Y, so grounding the un-rotated local cloud would seat the ceiling).
    let entity_rot = std::env::var("MARTIN_ROT")
        .ok()
        .and_then(|s| parse_euler_deg(&s))
        .unwrap_or_else(cloud_base_rotation);
    // Each part is resampled to the shared count N, then gets the *source* cloud it morphs in FROM,
    // chosen by its entrance. Build each part's `BuiltShot` directly (the only per-shot data the
    // director reads) — shape + morph-in origin + out-cloud + the resolved entrance/deform/raster/cue.
    let shots = build_shots(
        &parts,
        raws,
        &transitions,
        &deforms,
        &rasters,
        &additives,
        &starts,
        n,
        content_radius,
        entity_rot,
    );
    BuildOutput {
        shots,
        frame_center,
        content_radius,
        frame_factor,
        scene_norm,
        n,
        entity_rot,
    }
}

/// Once every referenced splat has loaded, build each part's shape (resampled to the fixed count) + the
/// intro ball off-thread (live) or inline (record/shot/bench — see `build_inline`), then `finalize_build`
/// uploads the initial window, spawns the single interpolate entity, and frames the union once.
#[allow(clippy::too_many_arguments)] // a Bevy system — params are injected, not a call-site burden
pub(crate) fn build_sequence(
    mut commands: Commands,
    mut assets: ResMut<Assets<PlanarGaussian3d>>,
    seq: Option<Res<Sequence>>,
    state: Option<ResMut<SeqState>>,
    root: Res<AssetRoot>,
    asset_server: Res<AssetServer>,
    mut cam: Query<&mut OrbitCam>,
    mut task: ResMut<SeqBuildTask>,
) {
    let (Some(seq), Some(mut state)) = (seq, state) else {
        return;
    };
    if state.built || seq.parts.is_empty() {
        return;
    }

    // a build is already in flight (live, off-thread) — poll it; finalize the frame it's ready.
    if let Some(t) = task.0.as_mut() {
        if let Some(out) = bevy::tasks::block_on(bevy::tasks::poll_once(t)) {
            task.0 = None;
            finalize_build(
                out,
                &mut commands,
                &mut assets,
                &asset_server,
                &mut cam,
                &seq,
                &mut state,
            );
        }
        return;
    }

    // Wait until every referenced splat has SETTLED — loaded OR failed. The old `assets.get(h).is_none()`
    // gate also blocked on `Failed` (whose `get()` stays `None` forever), so one missing/corrupt/typo'd
    // `.ply` hung the whole show indefinitely (no build, no frames, no exit — a disk-filling hang in the
    // exact `--record` mode used for the master). A failed load now falls through to the empty→transparent
    // -placeholder degrade in `build_shots`; the show plays on minus that part.
    {
        let mut failed = Vec::new();
        for (h, name) in state.loads.iter().zip(&state.load_names) {
            match asset_server.load_state(h) {
                bevy::asset::LoadState::Loaded => {}
                bevy::asset::LoadState::Failed(_) => failed.push(name.as_str()),
                _ => return, // NotLoaded / Loading — still in flight
            }
        }
        if !failed.is_empty() {
            warn!(
                "seq: {} splat(s) failed to load: {failed:?} — rendering without them",
                failed.len()
            );
        }
    }

    // Main thread, cheap: pre-extract the only asset-dependent content (`splat:` parts → an owned copy
    // out of `Assets`), so the worker build never touches the Bevy world. Non-splat parts sample in the
    // worker from fs+compute. `part_gaussians` ignores disk/aniso/count/font for `splat:`.
    let presampled: Vec<Option<Vec<Gaussian3d>>> = seq
        .parts
        .iter()
        .map(|p| {
            matches!(p.content, PartContent::Splats(_)).then(|| {
                part_gaussians(
                    &p.content,
                    &state,
                    &assets,
                    &root.0,
                    p.disk,
                    p.aniso,
                    None,
                    p.font.as_deref(),
                )
            })
        })
        .collect();
    let inputs = BuildInputs {
        parts: seq.parts.clone(),
        presampled,
        root: root.0.clone(),
        budget: seq.budget,
    };

    if build_inline() {
        // deterministic one-shot capture → build now, finalize this frame (no rendered frame before it).
        info!("reel build: inline (deterministic capture)");
        let out = build_cpu(inputs);
        finalize_build(
            out,
            &mut commands,
            &mut assets,
            &asset_server,
            &mut cam,
            &seq,
            &mut state,
        );
    } else {
        // live: hand the heavy build to a worker; the next frames poll it while the loader animates.
        info!("reel build: off-thread (loader animates while it builds)");
        task.0 =
            Some(bevy::tasks::AsyncComputeTaskPool::get().spawn(async move { build_cpu(inputs) }));
    }
}

/// Main-thread finalize of a completed `build_cpu`: upload the initial window, spawn the single morph
/// entity, frame the union, spawn any `glb:` dissolve meshes, and publish the shots into `SeqState`.
/// Everything here needs the Bevy world (assets/commands/camera) — kept off the worker on purpose.
#[allow(clippy::too_many_arguments)]
fn finalize_build(
    out: BuildOutput,
    commands: &mut Commands,
    assets: &mut Assets<PlanarGaussian3d>,
    asset_server: &AssetServer,
    cam: &mut Query<&mut OrbitCam>,
    seq: &Sequence,
    state: &mut SeqState,
) {
    let BuildOutput {
        mut shots,
        frame_center,
        content_radius,
        frame_factor,
        scene_norm,
        n,
        entity_rot,
    } = out;

    // STREAMING: upload only the initial window {0,1} now; the director streams the rest in/out.
    ensure_window(&mut shots, assets, 0);
    let intro0 = shots[0]
        .origin
        .clone()
        .expect("part 0 always builds a source cloud");

    // the reel sits at the world origin (parts carry their own @x,y,z).
    let reel_pos = Vec3::ZERO; // the reel sits at the origin; [stage] props carry their own @x,y,z

    let entity = commands
        .spawn((
            GaussianInterpolate::<Gaussian3d> {
                lhs: PlanarGaussian3dHandle(intro0.clone()),
                rhs: PlanarGaussian3dHandle(
                    shots[0]
                        .shape
                        .clone()
                        .expect("shot 0 shape loaded by ensure_window"),
                ),
            },
            CloudSettings {
                sort_mode: SortMode::Radix,
                radix_sort_depth_bits: crate::scene::sort_depth_bits(), // MARTIN_SORT_BITS (sort cost)
                time: 0.0, // start as the ball; shot_director morphs it in
                time_start: 0.0,
                time_stop: 1.0,
                bulge: 0.0,
                aabb: false, // round isotropic splats (OBB), not the conic-from-cov2d ellipse
                // MARTIN_SPLAT_SCALE shrinks every splat disk (default 1.0). Overdraw is the dominant
                // GPU cost, so <1 (e.g. 0.8) cuts fill on a weak GPU — at the cost of a sparser cloud.
                global_scale: crate::envvar::or("MARTIN_SPLAT_SCALE", 1.0_f32),
                // MARTIN_ADDITIVE / per-shot `additive:`: emissive One+One blend — overlapping
                // translucent splats GLOW on black instead of alpha-saturating to a solid blob (the
                // nebula/neon look). Off = alpha-over. Not set here (defaults false via `..default()`,
                // same as `rasterize_mode`) — `shot_director` applies the resolved per-shot value every
                // frame from `BuiltShot.additive`, including frame 0 (deterministic capture modes build
                // the whole reel inline before frame 0, so the director's first tick still lands before
                // any frame is written/captured).
                ..default()
            },
            Transform::from_rotation(entity_rot).with_translation(reel_pos),
        ))
        .id();

    // Frame the union once (camera never pops between parts); apply the same rotation to the centre
    // so the camera looks at the post-transform world centre. The seeding itself (MARTIN_ZOOM/YAW/
    // PITCH and the MARTIN_CAMERAS capture-pose override) lives in `camera::seed_orbit_framing`.
    let center = entity_rot * frame_center;
    for mut c in cam.iter_mut() {
        crate::camera::seed_orbit_framing(
            &mut c,
            center,
            content_radius,
            frame_factor,
            entity_rot,
            scene_norm,
        );
    }

    // `glb:` parts: render the real mesh AND sample its gaussians from that same mesh (filled by
    // sample_gl_mesh) so the mesh can dissolve into its own splats. Shares the cloud's base frame.
    for (idx, part) in seq.parts.iter().enumerate() {
        if let PartContent::GlMesh(name) = &part.content {
            // gl_dissolve needs this shape uploaded now + holds its own clone (kept alive even when the
            // streaming window later drops the BuiltShot's handle).
            let shape = shots[idx]
                .shape
                .clone()
                .unwrap_or_else(|| assets.add(shots[idx].shape_data.clone()));
            spawn_gl_dissolve(
                commands,
                asset_server,
                name,
                idx,
                entity_rot,
                part.rot.unwrap_or(Quat::IDENTITY),
                shape,
                n,
            );
        }
    }

    let built_n = shots.len();
    // PEAK resident with streaming = the heaviest active window {i-1, i, i+1} (shape + any origin/exit
    // source per shot in it), NOT every shot at once. Warn only if that window risks a GPU OOM.
    let per = |s: &BuiltShot| {
        (1 + usize::from(s.origin_data.is_some()) + usize::from(s.exit_data.is_some())) * n
    };
    let resident: usize = (0..shots.len())
        .map(|i| {
            let lo = i.saturating_sub(1);
            let hi = (i + 1).min(shots.len() - 1);
            (lo..=hi).map(|j| per(&shots[j])).sum()
        })
        .max()
        .unwrap_or(0);
    state.starts = shots.iter().map(|s| s.start).collect(); // cache the cue timeline once (immutable after)
    state.shots = shots;
    state.entity = Some(entity);
    state.built = true;
    // FREE THE SOURCE CAPTURES: the raw `.ply` clouds (held alive in `state.loads` until build) have
    // now been sampled into each shot's cloud — drop them so a big multi-capture demo doesn't hold the
    // huge source files in RAM on top of the built shots (the director reads `state.shots`, never loads).
    let freed = state.loads.len();
    state.loads.clear();
    state.load_names.clear();
    info!("sequence built: {built_n} shots × {n} gaussians (freed {freed} source clouds)");
    crate::scene::warn_splat_budget("sequence", resident);
}

/// Build each shot's `BuiltShot` (consuming `raws`) — its shape (resampled to the shared budget `n`,
/// with `rot:`/`tint:` baked in) + the morph-in `origin` cloud its entrance flows FROM (`~name` per
/// shot > Ball for shot 0 / Morph after; Morph/Swarm flow straight from the
/// previous shape with no source) + the `exit:` departure cloud + the resolved deform/raster/cue.
/// `MARTIN_PAIR=match` reorders each Morph shot so splat k slides to the nearest same-colour splat of
/// the PREVIOUS shape (cost = pos² + 0.5·colour²) — a coherent ghostly morph instead
/// of an index-rank centre-ball. The only per-shot data `shot_director` reads.
#[allow(clippy::too_many_arguments)] // the build inputs are genuinely independent resolved tables
fn build_shots(
    parts: &[Shot],
    raws: Vec<Vec<Gaussian3d>>,
    transitions: &[Entrance],
    deforms: &[Option<Deform>],
    rasters: &[RasterizeMode],
    additives: &[bool],
    starts: &[f32],
    n: usize,
    content_radius: f32,
    base_rot: Quat,
) -> Vec<BuiltShot> {
    let pair_match = std::env::var("MARTIN_PAIR")
        .map(|v| v.eq_ignore_ascii_case("match"))
        .unwrap_or(false);
    let pair_color_w = 0.5_f32; // colour weight in nearest-match pairing (was MARTIN_PAIR_COLOR)
    let mut shots: Vec<BuiltShot> = Vec::with_capacity(parts.len());
    let mut prev_shaped: Option<Vec<Gaussian3d>> = None;
    for (idx, raw) in raws.into_iter().enumerate() {
        // ROBUSTNESS: a part that produced 0 gaussians (an unsupported/broken asset) must NOT reach
        // the morph shader — an empty cloud is a wgpu validation error that crashes the whole render.
        // Degrade it to a transparent placeholder so the show plays on (the part is just invisible).
        let raw = if raw.is_empty() {
            warn!(
                "part {}: 0 gaussians — substituting a transparent placeholder (asset failed to load?)",
                parts[idx].content.label()
            );
            crate::mesh::transparent_placeholder(256)
        } else {
            raw
        };
        let mut shaped = resample_morton(raw, n);
        // bake the part's own `rot:` into its shape (so sources/out clouds inherit it, and the morph
        // between differently-oriented parts reorients smoothly). glb parts rotate in sample_gl_mesh.
        if let Some(q) = parts[idx].rot {
            if !matches!(parts[idx].content, PartContent::GlMesh(_)) {
                crate::morph::rotate_gaussians(&mut shaped, q);
            }
        }
        let tr = transitions[idx];
        let r = content_radius;
        // if the PREVIOUS part DEPARTS (washes/disperses away), there's no shape to flow from →
        // a Morph/Swarm part must assemble fresh from a ball instead.
        let prev_departs = idx > 0 && parts[idx - 1].exit.is_some();
        // pair=match: when this Morph part flows DIRECTLY from the previous shape (no departure, no
        // source cloud), reorder it so each splat slides to the nearest same-colour splat of the prev
        // part — minimal travel, coherent morph. Only the Morph/Swarm-flow case has a prev shape.
        if pair_match && matches!(tr, Entrance::Morph | Entrance::Swarm) && !prev_departs {
            if let Some(prev) = &prev_shaped {
                shaped = crate::morph::match_reorder(prev, shaped, pair_color_w);
            }
        }
        // `tint:` recolours this shot's shape (e.g. a deep-fried bitterbal) BEFORE the source/out
        // clouds and the next part's pair-match read it, so the whole lifecycle is tinted.
        if let Some(tint) = parts[idx].tint {
            crate::scene::colorize::apply(&mut shaped, tint);
        }
        // `ground:<y>` seats this part's lowest splat at world-Y (rest it on a [stage] plate). Baked
        // BEFORE the source/exit/ball clouds derive from `shaped`, so the whole lifecycle sits seated.
        if let Some(gy) = parts[idx].ground {
            crate::morph::ground_to(&mut shaped, base_rot, gy);
        }
        let src: Option<Vec<Gaussian3d>> = match tr {
            // Morph/Swarm flow from the PREVIOUS part's shape (no source) — unless the previous part
            // departed (washed away), in which case there's nothing to flow from → assemble fresh.
            Entrance::Morph | Entrance::Swarm if prev_departs => {
                Some(ball_of(&shaped, r * BALL_SHELL))
            }
            Entrance::Morph | Entrance::Swarm => None,
            // Cut shows the shape instantly (director pins the factor to 1.0) — flows from the prev
            // shape like Morph, no source cloud to build.
            Entrance::Cut => None,
            other => source_cloud(other, &shaped, r),
        };
        // `exit:` departure target cloud (faded + displaced) — the part morphs to this as it leaves.
        let exit = parts[idx].exit.map(|d| d.out_cloud(&shaped, r));
        // keep this part's shape so the NEXT part can pair-match against it (only needed for
        // pair=match; the clone is gated to avoid copying a big cloud on every render otherwise).
        if pair_match {
            prev_shaped = Some(shaped.clone());
        }
        let shot = &parts[idx];
        // STREAMING: build the CPU master clouds now; GPU handles stay None until `ensure_window`
        // uploads them (so a long reel never holds every shot's cloud in VRAM at once).
        shots.push(BuiltShot {
            shape_data: PlanarGaussian3d::from(shaped),
            origin_data: src.map(PlanarGaussian3d::from),
            exit_data: exit.map(PlanarGaussian3d::from),
            shape: None,
            origin: None,
            exit_cloud: None,
            entrance: tr,
            deform: deforms[idx],
            deform_amp: shot.deform_amp,
            deform_gated: shot.deform_gated,
            // a hard cut with no explicit flash gets an automatic white-flash pop on its frame.
            flash: shot.flash.or((tr == Entrance::Cut).then_some(0.8)),
            beat: shot.beat,
            raster: rasters[idx],
            additive: additives[idx],
            ease: shot.ease,
            freeze: shot.freeze,
            start: starts[idx],
            morph: shot.morph,
            bulge: shot.bulge,
            exit: shot.exit,
            is_gl_mesh: matches!(shot.content, PartContent::GlMesh(_)),
        });
    }
    shots
}

/// SPLAT STREAMING (core): keep only the shots in the active window `{idx-1, idx, idx+1}` uploaded as
/// GPU assets; drop the rest (the `Handle` goes `None` → the asset is freed → VRAM released). The CPU
/// master clouds stay in `*_data`, so re-entering the window just re-uploads. Uploads happen only on a
/// shot change (cheap), so a long reel of big captures never holds more than a few clouds in VRAM.
pub(crate) fn ensure_window(
    shots: &mut [BuiltShot],
    assets: &mut Assets<PlanarGaussian3d>,
    idx: usize,
) {
    let lo = idx.saturating_sub(1);
    let hi = (idx + 1).min(shots.len().saturating_sub(1));
    for (i, s) in shots.iter_mut().enumerate() {
        if i >= lo && i <= hi {
            if s.shape.is_none() {
                s.shape = Some(assets.add(s.shape_data.clone()));
            }
            if s.origin.is_none() {
                if let Some(d) = &s.origin_data {
                    s.origin = Some(assets.add(d.clone()));
                }
            }
            if s.exit_cloud.is_none() {
                if let Some(d) = &s.exit_data {
                    s.exit_cloud = Some(assets.add(d.clone()));
                }
            }
        } else {
            // outside the window → drop the GPU handles (asset freed). A `glb:` shape also has a
            // separate handle held by its gl_dissolve entity, so dropping here never breaks those.
            s.shape = None;
            s.origin = None;
            s.exit_cloud = None;
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_of_normalized_is_fixed_and_ignores_the_union() {
        // normalized shots always frame from the centroid origin at the fixed normalize radius — the
        // raw union (here lopsided with a far floater) must not move the framing.
        let (c, r, f) = frame_of(
            true,
            Vec3::new(-100.0, 0.0, 5.0),
            Vec3::new(100.0, 9.0, 9.0),
        );
        assert_eq!(c, Vec3::ZERO);
        assert_eq!(r, NORMALIZE_EXTENT * 0.5);
        assert_eq!(f, 2.5);
    }

    #[test]
    fn frame_of_raw_frames_the_union_box() {
        // raw mode centres on the union box and sizes the radius to its half-diagonal.
        let (c, r, f) = frame_of(false, Vec3::splat(-1.0), Vec3::splat(1.0));
        assert_eq!(c, Vec3::ZERO);
        assert!((r - 3.0_f32.sqrt()).abs() < 1e-5); // half-diagonal of a 2-unit cube
        assert_eq!(f, 1.7);
    }

    #[test]
    fn frame_of_raw_radius_has_a_floor() {
        // a degenerate (single-point) union must not yield a zero framing radius (camera-at-target).
        let (_, r, _) = frame_of(false, Vec3::ZERO, Vec3::ZERO);
        assert_eq!(r, 0.1);
    }

    #[test]
    fn arrival_entrance_shot0_cannot_flow_from_a_prev_shape() {
        // shot 0 has no previous shape — Morph/Swarm/Cut all degrade to a Ball assemble.
        assert_eq!(
            arrival_entrance(0, Some(Entrance::Morph), None),
            Entrance::Ball
        );
        assert_eq!(
            arrival_entrance(0, Some(Entrance::Swarm), None),
            Entrance::Ball
        );
        assert_eq!(
            arrival_entrance(0, Some(Entrance::Cut), None),
            Entrance::Ball
        );
        // an explicit non-flow entrance still applies on shot 0…
        assert_eq!(
            arrival_entrance(0, Some(Entrance::Fade), None),
            Entrance::Fade
        );
        // …and the default with nothing set is a Ball assemble.
        assert_eq!(arrival_entrance(0, None, None), Entrance::Ball);
    }

    #[test]
    fn arrival_entrance_precedence_and_later_default() {
        // explicit ~name beats the shot-0 Ball default.
        assert_eq!(
            arrival_entrance(2, Some(Entrance::Fade), Some(Entrance::Explode)),
            Entrance::Fade
        );
        // the global applies when the shot has no explicit entrance.
        assert_eq!(
            arrival_entrance(2, None, Some(Entrance::Explode)),
            Entrance::Explode
        );
        // later shots default to Morph (flow from the previous shape).
        assert_eq!(arrival_entrance(1, None, None), Entrance::Morph);
    }
}
