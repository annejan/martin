//! The one morph-timeline engine: a show is a list of `Part`s that each assemble in from a
//! source cloud (ball/fade/explode/… or a per-particle shader transition) and then hold,
//! morphing into the next. Drives a single `GaussianInterpolate` entity retargeted per part.

use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::prelude::*;
use bevy_gaussian_splatting::morph::interpolate::GaussianInterpolate;
use bevy_gaussian_splatting::sort::SortMode;
use bevy_gaussian_splatting::{
    CloudSettings, Gaussian3d, PlanarGaussian3d, PlanarGaussian3dHandle,
};

use crate::camera::{OrbitCam, DEFAULT_PITCH, FRONT_YAW};
use crate::morph::{ball_of, disperse_of, evaporate_of, resample_morton, sink_of, wash_of};
use crate::scene::content::{parse_source, part_gaussians, side_by_side, PartContent};
use crate::scene::effects::{source_cloud, Deform, Departure, Transition, BALL_SHELL};
use crate::scene::gl_dissolve::{gl_mesh_alpha, spawn_gl_dissolve};
use crate::scene::{cloud_base_rotation, file_name_of, parent_dir, AssetRoot, NORMALIZE_EXTENT};
use crate::score;
use crate::text::{build_text_outline_gaussians, build_text_penwrite_gaussians, TEXT_RGB};

const FLASH_LEN: f32 = 0.18; // cut-flash decay time (s), MARTIN_FLASH strength
const DEFORM_SPEED: f32 = 2.0; // deform animation rate: deform_time = clock.t * this
const DEPART_LEN: f32 = 1.5; // `out:` departure time (s) — carved from the end of a part's hold

/// One part morphs in from the previous (or, for part 0, from a ball), then holds.
#[derive(Clone)]
pub(crate) struct Part {
    pub content: PartContent,
    pub hold: f32,                      // seconds held after arriving
    pub morph: f32,                     // seconds to morph in
    pub bulge: f32,                     // ball-pulse explosiveness (Morph transition only)
    pub transition: Option<Transition>, // None = default (Ball for part 0, else Morph)
    pub anchor: Option<f32>,            // absolute start (s) on the music clock; None = relative
    pub deform: Option<Deform>, // persistent deform while held (None = none / MARTIN_DEFORM)
    pub out: Option<Departure>, // how the part LEAVES (`out:name`); None = cross-morph to the next
    pub rot: Option<Quat>,      // per-part orientation (`rot:rx,ry,rz` deg), baked into the shape
    pub cluster: Option<usize>, // `cluster:N` → N scattered, randomly-rotated copies (a "serving")
}

/// The whole show: a list of parts + the gaussian budget every part is resampled to.
#[derive(Resource)]
pub(crate) struct Sequence {
    pub parts: Vec<Part>,
    pub count: usize,
}

/// MARTIN_FLASH=<strength>: over-bright bloom pulse on each part cut (0 = off, the default).
#[derive(Resource)]
pub(crate) struct FlashStrength(pub f32);

/// Loaded splat handles + the per-part built shapes (all `count` gaussians) + each part's
/// morph-in source cloud + its resolved transition.
#[derive(Resource)]
pub(crate) struct SeqState {
    pub load_names: Vec<String>,
    pub loads: Vec<Handle<PlanarGaussian3d>>,
    pub shapes: Vec<Handle<PlanarGaussian3d>>,
    pub sources: Vec<Option<Handle<PlanarGaussian3d>>>, // per-part lhs source (None = morph from prev)
    pub out_clouds: Vec<Option<Handle<PlanarGaussian3d>>>, // per-part `out:` departure cloud (None = none)
    pub transitions: Vec<Transition>,                      // resolved transition per part
    pub deforms: Vec<Option<Deform>>,                      // resolved persistent deform per part
    pub starts: Vec<f32>,                                  // absolute start time (s) of each part
    pub built: bool,
    pub entity: Option<Entity>,
}

/// Load the capture-camera world positions from a 3DGS/COLMAP `cameras.json` (graphdeco format:
/// an array of objects each with a `"position": [x,y,z]`). These are in the same coordinates as
/// the scene's `.ply`, so applying the scene's normalize + cloud rotation places martin's camera
/// where the scene was actually shot — the only viewpoint a 360° capture renders coherently.
fn load_camera_positions(path: &str) -> Vec<Vec3> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    json.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|c| {
                    let p = c.get("position")?.as_array()?;
                    Some(Vec3::new(
                        p.first()?.as_f64()? as f32,
                        p.get(1)?.as_f64()? as f32,
                        p.get(2)?.as_f64()? as f32,
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse `MARTIN_SEQ`: a file path OR an inline string. Parts are `;`/newline-separated.
/// Each part: `text:STRING` or `splat:a.ply` (or `a.ply+b.ply` for side-by-side), optional
/// trailing `@hold,morph,bulge`. `#` comments and blank lines are skipped.
fn parse_seq(spec: &str, score: &score::Score) -> Vec<Part> {
    let raw = std::fs::read_to_string(spec).unwrap_or_else(|_| spec.to_string());
    let mut parts = Vec::new();
    // strip each line's `#` comment to end-of-line FIRST (so a `;` inside a comment can't split it
    // and leak the tail as a bogus part), then split into parts on `;`/newline.
    let cleaned: String = raw
        .lines()
        .map(|l| l.split('#').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    for line in cleaned.split([';', '\n']) {
        let s = line.trim();
        if s.is_empty() {
            continue;
        }
        // Pull the `~name` transition AND the `@@anchor` token (both single whitespace-delimited
        // tokens, position-independent); keep the rest of the line for the head + `@timing`.
        let mut transition = None;
        let mut anchor = None;
        let mut deform = None;
        let mut out = None;
        let mut rot = None;
        let mut cluster = None;
        // Pull each modifier token out of the line by its sigil/prefix. A token carrying a known
        // prefix is ALWAYS consumed (never leaks into the head/text) — if it fails to parse we warn,
        // so a typo (`~explod`, `^wave2`) is a visible error, not a silently-dropped effect.
        let s: String = s
            .split_whitespace()
            .filter(|tok| {
                if let Some(a) = tok.strip_prefix("@@") {
                    match score.anchor_seconds(a) {
                        Some(sec) => anchor = Some(sec),
                        None => {
                            eprintln!("seq: unknown anchor '@@{a}' (no such section/cue) — ignored")
                        }
                    }
                } else if let Some(d) = tok.strip_prefix("out:") {
                    match Departure::parse(d) {
                        Some(dep) => out = Some(dep),
                        None => eprintln!("seq: unknown departure 'out:{d}' — ignored"),
                    }
                } else if let Some(r) = tok.strip_prefix("rot:") {
                    match parse_euler_deg(r) {
                        Some(q) => rot = Some(q),
                        None => eprintln!("seq: bad 'rot:{r}' (need rx,ry,rz degrees) — ignored"),
                    }
                } else if let Some(c) = tok.strip_prefix("cluster:") {
                    match c.parse() {
                        Ok(n) => cluster = Some(n),
                        Err(_) => eprintln!("seq: bad 'cluster:{c}' (need an integer) — ignored"),
                    }
                } else if let Some(d) = tok.strip_prefix('^') {
                    match Deform::parse(d) {
                        Some(de) => deform = Some(de),
                        None => eprintln!("seq: unknown deform '^{d}' — ignored"),
                    }
                } else if let Some(t) = tok.strip_prefix('~') {
                    match Transition::parse(t) {
                        Some(tr) => transition = Some(tr),
                        None => eprintln!("seq: unknown transition '~{t}' — ignored"),
                    }
                } else {
                    return true; // not a modifier → keep it for the head + @timing
                }
                false // a modifier token → consume it
            })
            .collect::<Vec<_>>()
            .join(" ");
        let (head, timing) = match s.split_once('@') {
            Some((h, t)) => (h.trim(), Some(t.trim())),
            None => (s.as_str(), None),
        };
        let (mut hold, mut morph, mut bulge) = (1.5_f32, 3.0_f32, 0.9_f32);
        if let Some(t) = timing {
            let nums: Vec<f32> = t.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if let Some(v) = nums.first() {
                hold = *v;
            }
            if let Some(v) = nums.get(1) {
                morph = *v;
            }
            if let Some(v) = nums.get(2) {
                bulge = *v;
            }
        }
        let Some(content) = parse_source(head) else {
            eprintln!(
                "seq: unrecognized part '{head}' — expected one of \
                 text:/svg:/image:/mesh:/glb:/shader:/splat:/wall: — skipped"
            );
            continue;
        };
        parts.push(Part {
            content,
            hold,
            morph,
            bulge,
            transition,
            anchor,
            deform,
            out,
            rot,
            cluster,
        });
    }
    parts
}

/// Parse `rx,ry,rz` euler **degrees** into a quaternion (for a part's `rot:` token). Needs all three.
fn parse_euler_deg(s: &str) -> Option<Quat> {
    let n: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    (n.len() == 3).then(|| {
        Quat::from_euler(
            EulerRot::XYZ,
            n[0].to_radians(),
            n[1].to_radians(),
            n[2].to_radians(),
        )
    })
}

/// Build the show: `MARTIN_SEQ` if set, else a shorthand from `MARTIN_TEXT` /
/// `MARTIN_PLY(+_PLY2)(+_REFORM)`. Returns the sequence + the asset root (the .ply folder).
pub(crate) fn sequence_from_env(score: &score::Score) -> (Sequence, Option<String>) {
    // The default demo is now `assets/demo.show` (set as MARTIN_SHOW in `main` when nothing is
    // requested), so by the time we get here MARTIN_SEQ is set from its `[seq]` section.
    let count_default = if std::env::var("MARTIN_SEQ").is_ok() {
        200_000
    } else {
        0
    };
    let count = std::env::var("MARTIN_MORPH_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(count_default);

    if let Ok(spec) = std::env::var("MARTIN_SEQ") {
        // asset root = the .ply folder (so `splat:` filenames resolve); MARTIN_PLY sets it.
        let root = std::env::var("MARTIN_PLY").ok().and_then(parent_dir);
        return (
            Sequence {
                parts: parse_seq(&spec, score),
                count,
            },
            root,
        );
    }

    if let Ok(text) = std::env::var("MARTIN_TEXT") {
        let part = Part {
            content: PartContent::Text(text),
            hold: 2.0,
            morph: 3.0,
            bulge: 0.0,
            transition: None,
            anchor: None,
            deform: None,
            out: None,
            rot: None,
            cluster: None,
        };
        return (
            Sequence {
                parts: vec![part],
                count,
            },
            None,
        );
    }

    // splat shorthand: PLY (+ PLY2) as part 0; REFORM (if any) as part 1.
    let primary = std::env::var("MARTIN_PLY").ok();
    let root = primary.as_deref().and_then(|p| parent_dir(p.to_string()));
    let name1 = primary
        .as_deref()
        .map(file_name_of)
        .unwrap_or_else(|| "aegg.ply".into());
    let mut names = vec![name1];
    if let Ok(p2) = std::env::var("MARTIN_PLY2") {
        names.push(file_name_of(&p2));
    }
    let bulge = std::env::var("MARTIN_BULGE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.9);
    let mut parts = vec![Part {
        content: PartContent::Splats(side_by_side(names.iter().map(String::as_str))),
        hold: 2.0,
        morph: 3.0,
        bulge: 0.0,
        transition: None,
        anchor: None,
        deform: None,
        out: None,
        rot: None,
        cluster: None,
    }];
    if let Ok(reform) = std::env::var("MARTIN_REFORM") {
        parts.push(Part {
            content: PartContent::Splats(vec![(file_name_of(&reform), Vec3::ZERO)]),
            hold: 2.0,
            morph: 3.5,
            bulge,
            transition: None,
            anchor: None,
            deform: None,
            out: None,
            rot: None,
            cluster: None,
        });
    }
    (Sequence { parts, count }, root)
}

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

    // Absolute start time (s) of each part — the cue timeline (anchors, else laid end-to-end).
    let starts = part_starts(&seq.parts);

    // read every part's gaussians once, so count==0 can mean "size N to the largest part"
    // (every part is then resampled to that single N — required by the shared morph output).
    // pen-write strokes are thin: MARTIN_PW_SPLAT (gaussian size) / MARTIN_PW_STEP (sample
    // spacing) tune stroke weight — a fat splat blooms the strokes into filled blobs.
    let pw_step = std::env::var("MARTIN_PW_STEP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.5_f32);
    let pw_splat = std::env::var("MARTIN_PW_SPLAT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0.006_f32);
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
    let cluster_total = std::env::var("MARTIN_MORPH_COUNT")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(200_000);
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
    let n = if seq.count > 0 {
        seq.count
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
    for (idx, raw) in raws.into_iter().enumerate() {
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
        let out = seq.parts[idx].out.map(|d| match d {
            Departure::Wash => wash_of(&shaped, r * 2.5),
            Departure::Disperse => disperse_of(&shaped, r * 1.8),
            Departure::Evaporate => evaporate_of(&shaped, r * 3.0),
            Departure::Sink => sink_of(&shaped, r * 3.0),
        });
        sources.push(src.map(|s| assets.add(PlanarGaussian3d::from(s))));
        out_clouds.push(out.map(|o| assets.add(PlanarGaussian3d::from(o))));
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
        .and_then(|s| {
            let n: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            (n.len() == 3).then(|| {
                Quat::from_euler(
                    EulerRot::XYZ,
                    n[0].to_radians(),
                    n[1].to_radians(),
                    n[2].to_radians(),
                )
            })
        })
        .unwrap_or_else(cloud_base_rotation);

    let entity = commands
        .spawn((
            GaussianInterpolate::<Gaussian3d> {
                lhs: PlanarGaussian3dHandle(intro0.clone()),
                rhs: PlanarGaussian3dHandle(shapes[0].clone()),
            },
            CloudSettings {
                sort_mode: SortMode::Radix,
                time: 0.0, // start as the ball; part_director morphs it in
                time_start: 0.0,
                time_stop: 1.0,
                bulge: 0.0,
                ..default()
            },
            Transform::from_rotation(entity_rot),
        ))
        .id();

    // frame the union once (camera never pops between parts); apply the same rotation to the
    // centre so the camera looks at the post-transform world centre.
    // Seed the free-orbit camera. MARTIN_ZOOM scales distance (>1 = closer); MARTIN_YAW /
    // MARTIN_PITCH (radians) seed the orbit angle so you can bake a found viewpoint into a
    // render (and freely orbit live from there).
    let env_f = |k: &str| std::env::var(k).ok().and_then(|s| s.parse::<f32>().ok());
    let zoom = env_f("MARTIN_ZOOM").filter(|z| *z > 0.0).unwrap_or(1.0);
    let center = entity_rot * frame_center;
    let (mut yaw, mut pitch, mut dist) = (
        env_f("MARTIN_YAW").unwrap_or(FRONT_YAW),
        env_f("MARTIN_PITCH").unwrap_or(DEFAULT_PITCH),
        content_radius * frame_factor / zoom,
    );
    // MARTIN_CAMERAS=<cameras.json>: park the camera at a real capture pose (the only viewpoint
    // a raw 360° scene renders coherently). Transform the chosen capture position through the
    // SAME normalize (part 0) + cloud rotation as the gaussians, then read off yaw/pitch/dist
    // around the framed centre. MARTIN_CAM_INDEX picks which shot (default 0).
    if let Ok(cpath) = std::env::var("MARTIN_CAMERAS") {
        let positions = load_camera_positions(&cpath);
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

    state.shapes = shapes;
    state.sources = sources;
    state.out_clouds = out_clouds;
    state.transitions = transitions;
    state.deforms = deforms;
    state.starts = starts;
    state.entity = Some(entity);
    state.built = true;
    info!(
        "sequence built: {} parts × {n} gaussians",
        state.shapes.len()
    );
}

/// Index of the active part at time `t`: the last part whose absolute start (from the cue
/// timeline — `@@anchor` or laid end-to-end) has arrived. Shared by `part_director` and `flypath`.
pub(crate) fn active_part(starts: &[f32], t: f32) -> usize {
    let mut idx = 0;
    for (i, &start) in starts.iter().enumerate() {
        if t >= start {
            idx = i;
        } else {
            break;
        }
    }
    idx
}

/// Absolute start time (s) of each part: its `@@anchor` (locked to the music clock) if set, else
/// laid end-to-end after the previous part (`prev.start + prev.morph + prev.hold`). The cue
/// timeline — shared by `build_sequence` and the `MARTIN_VALIDATE` dry-run.
pub(crate) fn part_starts(parts: &[Part]) -> Vec<f32> {
    let mut starts = Vec::with_capacity(parts.len());
    let mut cursor = 0.0_f32;
    for (i, part) in parts.iter().enumerate() {
        let start = part.anchor.unwrap_or(if i == 0 { 0.0 } else { cursor });
        starts.push(start);
        cursor = start + part.morph + part.hold;
    }
    starts
}

/// End of the cue timeline: the latest part's `start + morph + hold` (anchors can push it past a
/// simple sum). The recorder uses this (+ a tail) for the clip length; `flypath` spreads the
/// camera path across it while recording.
pub(crate) fn show_end(parts: &[Part], starts: &[f32]) -> f32 {
    parts
        .iter()
        .zip(starts)
        .map(|(p, &start)| start + p.morph + p.hold)
        .fold(0.0_f32, f32::max)
}

/// Drive the show from `SeqClock.t`: find the active part, retarget the interpolate entity's
/// lhs/rhs (only on change), and set the blend factor + ball bulge. Part 0 morphs in from the
/// intro ball; every later part morphs in from the previous part's shape.
#[allow(clippy::type_complexity)]
pub(crate) fn part_director(
    seq: Option<Res<Sequence>>,
    state: Option<Res<SeqState>>,
    clock: Res<crate::scene::SeqClock>,
    flash: Res<FlashStrength>,
    beat: Res<crate::scene::beat::Beat>,
    // (amp_scale, speed) for the persistent deform — read from env once. MARTIN_DEFORM_AMP scales the
    // wobble strength (e.g. 0.3 = gentle on a whole scene), MARTIN_DEFORM_SPEED its rate.
    mut deform_tune: Local<Option<(f32, f32)>>,
    mut q: Query<(
        &mut GaussianInterpolate<Gaussian3d>,
        &mut CloudSettings,
        &mut Transform,
    )>,
) {
    let (Some(seq), Some(state)) = (seq, state) else {
        return;
    };
    if !state.built {
        return;
    }
    let Some(entity) = state.entity else { return };
    let Ok((mut interp, mut cs, mut tf)) = q.get_mut(entity) else {
        return;
    };
    let parts = &seq.parts;

    // The active part is the last one whose absolute start time has arrived (starts come from
    // the cue timeline — `@@anchor` or laid end-to-end). It morphs in over `morph`, then holds
    // until the next part starts. Before part 0's start, `factor` clamps to 0 (its source state).
    let t = clock.t;
    let starts = &state.starts;
    let idx = active_part(starts, t);
    // Phase: ARRIVING (source → shape), holding (shape), or DEPARTING (shape → its faded out-cloud
    // — a distinct step carved from the end of the hold, before the next part arrives; see `out:`).
    let next = idx + 1;
    let depart_at = if next < parts.len() && parts[idx].out.is_some() {
        (starts[next] - DEPART_LEN).max(starts[idx] + parts[idx].morph)
    } else {
        f32::MAX
    };
    let departing = t >= depart_at;
    let (want_lhs, want_rhs, factor, arriving) = if departing {
        let f = ((t - depart_at) / DEPART_LEN).clamp(0.0, 1.0);
        let out = state.out_clouds[idx].as_ref().unwrap_or(&state.shapes[idx]);
        (&state.shapes[idx], out, f, false)
    } else {
        let dt = t - starts[idx];
        let f = (dt / parts[idx].morph.max(1e-3)).clamp(0.0, 1.0);
        // lhs: the part's source cloud (ball/fade/explode/…), or — for a plain Morph — the prev shape.
        let lhs = match &state.sources[idx] {
            Some(h) => h,
            None => &state.shapes[idx - 1],
        };
        (lhs, &state.shapes[idx], f, dt < parts[idx].morph)
    };
    if interp.lhs.0.id() != want_lhs.id() {
        interp.lhs = PlanarGaussian3dHandle(want_lhs.clone());
    }
    if interp.rhs.0.id() != want_rhs.id() {
        interp.rhs = PlanarGaussian3dHandle(want_rhs.clone());
    }
    let morphing = arriving || departing;
    let eased = factor * factor * (3.0 - 2.0 * factor);
    cs.time = eased;
    // the ball-pulse shader effect belongs to the plain Morph transition (prev → next through a
    // ball); source-based transitions carry their own motion, so they don't pulse.
    cs.bulge = if arriving && state.transitions[idx] == Transition::Morph {
        parts[idx].bulge
    } else {
        0.0
    };
    // ~swarm: flock the particles along curled paths during the morph (the @_,_,N timing value is
    // the swarm strength); mutually exclusive with the ball-pulse above.
    cs.swarm = if arriving && state.transitions[idx] == Transition::Swarm {
        parts[idx].bulge
    } else {
        0.0
    };
    // per-particle shader transitions (typewriter/sparkle/…): drive the vendored uniforms only
    // while morphing in; otherwise mode 0 = off (held shape renders plain, fully sort-safe).
    let (mode, soft, axis) = arriving
        .then(|| state.transitions[idx].shader_uniforms())
        .flatten()
        .unwrap_or((0, 0.0, 0));
    cs.transition_mode = mode;
    cs.transition_softness = soft;
    cs.transition_axis = axis;
    // Persistent deform (wave/cloth/ripple/twist): unlike the transition this runs the *whole*
    // time the part is up (not just while morphing), animated by the show clock. Mode 0 = off.
    let (amp_scale, speed) = *deform_tune.get_or_insert_with(|| {
        let f = |k: &str, d: f32| {
            std::env::var(k)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(d)
        };
        (
            f("MARTIN_DEFORM_AMP", 1.0),
            f("MARTIN_DEFORM_SPEED", DEFORM_SPEED),
        )
    });
    let (dmode, damp, dfreq) = state.deforms[idx]
        .map(|d| d.uniforms())
        .unwrap_or((0, 0.0, 0.0));
    cs.deform_mode = dmode;
    cs.deform_amp = damp * amp_scale;
    cs.deform_freq = dfreq;
    cs.deform_time = t * speed;
    // Flash on each cut (term-demo's Director trick): a brief over-bright pulse at every part
    // start → the HDR bloom flares. MARTIN_FLASH=<strength> (0 = off, default); reuses
    // global_opacity, so off keeps every frame byte-identical.
    // MARTIN_FLASH defaults to 0 — skip the per-frame max-over-starts loop entirely in that case.
    let flash = if flash.0 <= 0.0 {
        0.0
    } else {
        flash.0
            * starts
                .iter()
                .map(|&s| {
                    let d = t - s;
                    if (0.0..FLASH_LEN).contains(&d) {
                        let a = 1.0 - d / FLASH_LEN;
                        a * a
                    } else {
                        0.0
                    }
                })
                .fold(0.0_f32, f32::max)
    };
    cs.global_opacity = 1.0 + flash;

    // Beat reactions (MARTIN_BEAT scale): the score's drum hits drive the look. A held part can't
    // use `bulge` (it's a mid-morph ball-pulse, zero at time==1), so the kick thump rides on the
    // cloud's scale; the snare flares the bloom; kick+snare swell any active deform so a ^wave /
    // ^ripple part pumps with the track. During a morph we add a little bulge punch too.
    let k = beat.intensity;
    if k > 0.0 {
        tf.scale = Vec3::splat(1.0 + beat.kick * 0.05 * k);
        cs.global_opacity += (beat.snare * 0.45 + beat.hat * 0.12) * k;
        if morphing {
            cs.bulge += beat.kick * 0.3 * k;
        }
        if cs.deform_mode != 0 {
            cs.deform_amp *= 1.0 + (beat.kick * 0.6 + beat.snare * 0.3) * k;
        }
    }

    // glb: dissolve — the splats are the exact complement of the mesh (1 − its alpha): present
    // during the splat-assemble, hidden while the mesh is crisp (no poke-through), and back as it
    // dissolves — so mesh↔splats crossfade, and the dissolve completes before the next part morphs.
    if matches!(parts[idx].content, PartContent::GlMesh(_)) {
        cs.global_opacity *= 1.0 - gl_mesh_alpha(starts, parts, idx, t);
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

    fn parts(spec: &str) -> Vec<Part> {
        parse_seq(spec, &score::Score::builtin())
    }

    #[test]
    fn parse_seq_reads_heads_timing_and_modifiers() {
        let p = parts("text:HELLO @4,2 ~fade ^wave out:sink rot:0,90,0 cluster:3");
        assert_eq!(p.len(), 1);
        assert!(matches!(&p[0].content, PartContent::Text(s) if s == "HELLO"));
        assert_eq!(p[0].hold, 4.0);
        assert_eq!(p[0].morph, 2.0);
        assert_eq!(p[0].transition, Some(Transition::Fade));
        assert_eq!(p[0].deform, Some(Deform::Wave));
        assert_eq!(p[0].out, Some(Departure::Sink));
        assert_eq!(p[0].cluster, Some(3));
        assert!(p[0].rot.is_some());
    }

    #[test]
    fn parse_seq_splits_parts_and_skips_unknown_heads() {
        // `txet:` is a typo → that part is skipped (warned), the others survive.
        let p = parts("text:A; txet:B; text:C");
        assert_eq!(p.len(), 2);
        assert!(matches!(&p[0].content, PartContent::Text(s) if s == "A"));
        assert!(matches!(&p[1].content, PartContent::Text(s) if s == "C"));
    }

    #[test]
    fn unknown_modifier_is_consumed_not_leaked_into_the_head() {
        // a typo'd transition must NOT end up as part of the text.
        let p = parts("text:HELLO ~explod");
        assert_eq!(p.len(), 1);
        assert!(matches!(&p[0].content, PartContent::Text(s) if s == "HELLO"));
        assert_eq!(p[0].transition, None);
    }

    #[test]
    fn comment_with_a_semicolon_does_not_resurrect_a_bogus_part() {
        // regression: a `;` inside a `#` comment used to split it and parse the tail as a part.
        let p = parts("text:A   # note; with a ~semicolon and ~fade inside\ntext:B");
        assert_eq!(p.len(), 2);
        assert!(matches!(&p[0].content, PartContent::Text(s) if s == "A"));
        assert!(matches!(&p[1].content, PartContent::Text(s) if s == "B"));
        assert_eq!(p[0].transition, None); // the ~fade was inside the comment
    }

    #[test]
    fn part_starts_lay_end_to_end_then_honour_anchors() {
        let p = parts("text:A @2,1; text:B @3,1; text:C @1,1");
        let s = part_starts(&p);
        assert_eq!(s[0], 0.0);
        assert_eq!(s[1], 3.0); // 0 + morph 1 + hold 2
        assert_eq!(s[2], 7.0); // 3 + 1 + 3
        assert_eq!(show_end(&p, &s), 9.0); // 7 + 1 + 1
    }

    #[test]
    fn active_part_picks_the_latest_started() {
        let starts = [0.0, 3.0, 7.0];
        assert_eq!(active_part(&starts, 0.0), 0);
        assert_eq!(active_part(&starts, 2.9), 0);
        assert_eq!(active_part(&starts, 3.0), 1);
        assert_eq!(active_part(&starts, 100.0), 2);
    }

    #[test]
    fn parse_euler_deg_needs_three_components() {
        assert!(parse_euler_deg("0,90,0").is_some());
        assert!(parse_euler_deg("0,90").is_none());
        assert!(parse_euler_deg("x,y,z").is_none());
    }
}
