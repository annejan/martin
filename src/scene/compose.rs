//! The composition stage (`MARTIN_COMPOSE`): many objects on one stage at once, each placed +
//! spinning/swaying/bobbing/drifting, fading in on the music — vs the single-morph timeline.

use std::f32::consts::PI;

use bevy::prelude::*;
use bevy_gaussian_splatting::morph::interpolate::GaussianInterpolate;
use bevy_gaussian_splatting::sort::SortMode;
use bevy_gaussian_splatting::{
    CloudSettings, Gaussian3d, PlanarGaussian3d, PlanarGaussian3dHandle,
};

use crate::camera::{OrbitCam, DEFAULT_PITCH, FRONT_YAW};
use crate::capture::RecordState;
use crate::morph::resample_morton;
use crate::scene::content::{parse_source, part_gaussians, PartContent};
use crate::scene::sequence::SeqState;
use crate::scene::{cloud_base_rotation, AssetRoot, SeqClock, NORMALIZE_EXTENT};
use crate::score;

/// One object placed on the composition stage: a source + where it sits + how it moves.
#[derive(Clone)]
pub(crate) struct Composed {
    content: PartContent,
    pos: Vec3,
    scale: f32,
    rot: Vec3,   // static orientation, euler degrees
    spin: Vec3,  // auto-rotation, degrees/sec
    sway: Vec3, // oscillating rotation amplitude, degrees (swings front-on; for hollow-back splats)
    bob: f32,   // vertical bob amplitude (units)
    drift: Vec3, // translation velocity (units/sec)
    appear: f32, // fade-in start (s on the show clock)
    out: f32,   // fade-out start (s); f32::MAX = stays to the end
    fade: f32,  // fade in/out duration (s)
}

impl Composed {
    /// The object's source content — so the splat loader can collect its `splat:` filenames.
    pub(crate) fn content(&self) -> &PartContent {
        &self.content
    }
}

impl Composition {
    /// How long to record a composition stage: enough for every object to have appeared (and any
    /// fade-out to finish), plus a tail. The recorder uses this since a compose stage has no morph
    /// timeline to derive an end from.
    pub(crate) fn record_secs(&self) -> f32 {
        let mut end = 0.0_f32;
        for o in &self.objects {
            end = end.max(o.appear.max(0.0));
            if o.out < f32::MAX {
                end = end.max(o.out + o.fade);
            }
        }
        (end + 8.0).max(12.0)
    }
}

/// `MARTIN_COMPOSE=<file>`: a stage of objects, all on screen together.
#[derive(Resource)]
pub(crate) struct Composition {
    pub objects: Vec<Composed>,
    pub built: bool,
}

/// Per-object animation state, carried on each spawned cloud entity.
#[derive(Component)]
pub(crate) struct ComposeAnim {
    base_pos: Vec3,
    base_rot: Quat,
    base_scale: f32,
    spin: Vec3, // rad/sec
    sway: Vec3, // rad amplitude, oscillating
    bob: f32,
    drift: Vec3,
    appear: f32,
    out: f32,
    fade: f32,
}

/// Parse `MARTIN_COMPOSE` (a file path or inline string). Each line: a `<source>` head (text/splat/
/// mesh/image) then placement tokens — `@x,y,z` position, `*scale`, `rot a,b,c`, `spin a,b,c`
/// (deg/s), `sway a,b,c` (deg), `bob amp`, `drift dx,dy,dz`, `in/out <anchor>` (section/bar/beat/s).
pub(crate) fn parse_compose(spec: &str, score: &score::Score) -> Vec<Composed> {
    let raw = std::fs::read_to_string(spec).unwrap_or_else(|_| spec.to_string());
    let kw = |t: &str| matches!(t, "rot" | "spin" | "sway" | "bob" | "drift" | "in" | "out");
    let mut out = Vec::new();
    for line in raw.split([';', '\n']) {
        let s = line.split('#').next().unwrap_or("").trim();
        if s.is_empty() {
            continue;
        }
        let toks: Vec<&str> = s.split_whitespace().collect();
        // source = the leading tokens up to the first placement token (so `text:HELLO WORLD` works).
        let split = toks
            .iter()
            .position(|t| t.starts_with('@') || t.starts_with('*') || kw(t))
            .unwrap_or(toks.len());
        let Some(content) = parse_source(&toks[..split].join(" ")) else {
            continue;
        };
        let rest = &toks[split..];
        let (mut pos, mut scale, mut rot) = (Vec3::ZERO, 1.0_f32, Vec3::ZERO);
        let (mut spin, mut sway, mut bob, mut drift) =
            (Vec3::ZERO, Vec3::ZERO, 0.0_f32, Vec3::ZERO);
        // appear < 0 = no fade-in (visible from the start); `in <anchor>` sets it to a real time.
        let (mut appear, mut out_t, fade) = (-1.0_f32, f32::MAX, 0.8_f32);
        let mut i = 0;
        while i < rest.len() {
            let t = rest[i];
            if let Some(v) = t.strip_prefix('@') {
                pos = vec3_csv(v);
                i += 1;
            } else if let Some(v) = t.strip_prefix('*') {
                scale = v.parse().unwrap_or(1.0);
                i += 1;
            } else {
                let val = rest.get(i + 1).copied().unwrap_or("");
                match t {
                    "rot" => rot = vec3_csv(val),
                    "spin" => spin = vec3_csv(val),
                    "sway" => sway = vec3_csv(val),
                    "drift" => drift = vec3_csv(val),
                    "bob" => bob = val.parse().unwrap_or(0.0),
                    "in" => appear = score.anchor_seconds(val).unwrap_or(0.0),
                    "out" => out_t = score.anchor_seconds(val).unwrap_or(f32::MAX),
                    _ => {}
                }
                i += 2;
            }
        }
        out.push(Composed {
            content,
            pos,
            scale,
            rot,
            spin,
            sway,
            bob,
            drift,
            appear,
            out: out_t,
            fade,
        });
    }
    out
}

fn vec3_csv(s: &str) -> Vec3 {
    let n: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    Vec3::new(
        n.first().copied().unwrap_or(0.0),
        n.get(1).copied().unwrap_or(0.0),
        n.get(2).copied().unwrap_or(0.0),
    )
}

/// Build the stage once every referenced splat has loaded: each object → its own gaussian cloud
/// entity placed at its transform with a `ComposeAnim` for motion. Frames the camera on the union.
pub(crate) fn build_composition(
    mut commands: Commands,
    mut assets: ResMut<Assets<PlanarGaussian3d>>,
    comp: Option<ResMut<Composition>>,
    state: Option<Res<SeqState>>,
    root: Res<AssetRoot>,
    mut cam: Query<&mut OrbitCam>,
) {
    let (Some(mut comp), Some(state)) = (comp, state) else {
        return;
    };
    if comp.built || comp.objects.is_empty() {
        return;
    }
    if state.loads.iter().any(|h| assets.get(h).is_none()) {
        return; // wait for every referenced splat
    }
    let base = cloud_base_rotation();
    // cap each object's splats so a stage of big splats stays performant on the iGPU.
    let count = std::env::var("MARTIN_MORPH_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(120_000);
    let mut placed: Vec<(Vec3, f32)> = Vec::new(); // (centre, radius) per object, for framing
    for obj in &comp.objects {
        let mut raw = part_gaussians(&obj.content, &state, &assets, &root.0);
        if raw.is_empty() {
            continue;
        }
        crate::morph::normalize_to(&mut raw, NORMALIZE_EXTENT); // centre + ~2 units across
        let raw = resample_morton(raw, count);
        let rot = Quat::from_euler(
            EulerRot::XYZ,
            obj.rot.x.to_radians(),
            obj.rot.y.to_radians(),
            obj.rot.z.to_radians(),
        ) * base;
        let handle = assets.add(PlanarGaussian3d::from(raw));
        commands.spawn((
            // a static GaussianInterpolate (lhs == rhs) — the same render path the morph engine
            // uses (a plain PlanarGaussian3dHandle isn't picked up by martin's pipeline).
            // NB: do NOT add NoFrustumCulling here — `calculate_bounds` skips culling-exempt
            // entities, so they'd never get an Aabb and `extract_gaussians` would drop them
            // (black screen). Static stage clouds want frustum culling anyway.
            GaussianInterpolate::<Gaussian3d> {
                lhs: PlanarGaussian3dHandle(handle.clone()),
                rhs: PlanarGaussian3dHandle(handle),
            },
            CloudSettings {
                sort_mode: SortMode::Radix,
                time: 0.0,
                time_start: 0.0,
                time_stop: 1.0,
                bulge: 0.0,
                global_opacity: 0.0, // animate_composition fades it in
                ..default()
            },
            Transform {
                translation: obj.pos,
                rotation: rot,
                scale: Vec3::splat(obj.scale),
            },
            ComposeAnim {
                base_pos: obj.pos,
                base_rot: rot,
                base_scale: obj.scale,
                spin: obj.spin * (PI / 180.0),
                sway: obj.sway * (PI / 180.0),
                bob: obj.bob,
                drift: obj.drift,
                appear: obj.appear,
                out: obj.out,
                fade: obj.fade,
            },
        ));
        placed.push((obj.pos, NORMALIZE_EXTENT * 0.5 * obj.scale));
    }
    comp.built = true;
    if placed.is_empty() {
        return;
    }
    let center = placed.iter().map(|(p, _)| *p).sum::<Vec3>() / placed.len() as f32;
    let radius = placed
        .iter()
        .map(|(p, r)| (*p - center).length() + r)
        .fold(0.1_f32, f32::max);
    let zoom = std::env::var("MARTIN_ZOOM")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .filter(|z| *z > 0.0)
        .unwrap_or(1.0);
    let dist = radius * 2.5 / zoom;
    for mut c in &mut cam {
        c.target = center;
        c.dist = dist;
        c.yaw = FRONT_YAW;
        c.pitch = DEFAULT_PITCH;
        c.framed = true;
    }
    info!(
        "composition: {} objects on stage, centre [{:.2},{:.2},{:.2}], dist {:.2}",
        placed.len(),
        center.x,
        center.y,
        center.z,
        dist
    );
}

/// Animate the stage from the show clock: spin + bob + drift each object, fade it in (and out, if
/// it has an `out` time) via `global_opacity`.
pub(crate) fn animate_composition(
    clock: Res<SeqClock>,
    beat: Res<crate::scene::beat::Beat>,
    mut q: Query<(&ComposeAnim, &mut Transform, &mut CloudSettings)>,
) {
    let t = clock.t;
    let k = beat.intensity;
    for (a, mut tf, mut cs) in &mut q {
        // spin = continuous rotation; sway = a gentle oscillation around the base orientation
        // (swings a hollow-back single-image splat left/right without ever facing away).
        let osc = (t * 0.6).sin();
        tf.rotation = a.base_rot
            * Quat::from_euler(
                EulerRot::XYZ,
                a.spin.x * t + a.sway.x * osc,
                a.spin.y * t + a.sway.y * osc,
                a.spin.z * t + a.sway.z * osc,
            );
        let bob = if a.bob != 0.0 {
            a.bob * (t * 1.5).sin()
        } else {
            0.0
        };
        tf.translation = a.base_pos + a.drift * t + Vec3::Y * bob;
        // kick thumps the object's scale (bulge is a no-op on a static cloud, so scale carries it).
        tf.scale = Vec3::splat(a.base_scale * (1.0 + beat.kick * 0.06 * k));
        let fin = if a.appear < 0.0 {
            1.0 // no `in` → visible from the start (robust even if the clock hasn't advanced)
        } else {
            ((t - a.appear) / a.fade.max(1e-3)).clamp(0.0, 1.0)
        };
        let fout = if a.out < f32::MAX {
            ((a.out + a.fade - t) / a.fade.max(1e-3)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        // snare flares the bloom, hat shimmers — scaled by current visibility so beats don't
        // fight the fade-in.
        let vis = fin.min(fout);
        cs.global_opacity = vis * (1.0 + (beat.snare * 0.4 + beat.hat * 0.12) * k);
    }
}

/// Slowly orbit the camera around the stage (the "flow") — additive with the live arrow keys.
pub(crate) fn compose_camera(
    comp: Option<Res<Composition>>,
    rec: Res<RecordState>,
    time: Res<Time>,
    mut cam: Query<&mut OrbitCam>,
) {
    if comp.map(|c| c.built).unwrap_or(false) {
        let dt = if rec.dir.is_some() {
            1.0 / 60.0
        } else {
            time.delta_secs()
        };
        for mut c in &mut cam {
            c.yaw += 0.12 * dt;
        }
    }
}
