//! The composition stage (`MARTIN_COMPOSE`): many objects on one stage at once, each placed +
//! spinning/swaying/bobbing/drifting, fading in on the music — vs the single-morph timeline.

use std::f32::consts::PI;

use bevy::gltf::GltfAssetLabel;
use bevy::prelude::*;
use bevy_gaussian_splatting::morph::interpolate::GaussianInterpolate;
use bevy_gaussian_splatting::sort::SortMode;
use bevy_gaussian_splatting::{
    CloudSettings, Gaussian3d, PlanarGaussian3d, PlanarGaussian3dHandle,
};

use crate::camera::{DEFAULT_PITCH, FRONT_YAW, OrbitCam};
use crate::capture::RecordState;
use crate::morph::resample_morton;
use crate::scene::content::{PartContent, parse_source, part_gaussians};
use crate::scene::effects::{Deform, Transition, source_cloud};
use crate::scene::sequence::{SeqState, Sequence};
use crate::scene::{AssetRoot, NORMALIZE_EXTENT, SeqClock, cloud_base_rotation};
use crate::score;

const COMPOSE_MORPH: f32 = 3.6; // how long a `~transition` compose object takes to assemble in (s) —
// long enough that a pen-write breathes as the letters are drawn in stroke by stroke

/// One object placed on the composition stage: a source + where it sits + how it moves.
#[derive(Clone)]
pub(crate) struct Prop {
    content: PartContent,
    pos: Vec3,
    scale: f32,
    rot: Vec3,                                  // static orientation, euler degrees
    spin: Vec3,                                 // auto-rotation, degrees/sec
    sway: Vec3, // oscillating rotation amplitude, degrees (swings front-on; for hollow-back splats)
    bob: f32,   // vertical bob amplitude (units)
    drift: Vec3, // translation velocity (units/sec)
    appear: f32, // fade-in start (s on the show clock)
    out: f32,   // fade-out start (s); f32::MAX = stays to the end
    fade: f32,  // fade in/out duration (s)
    transition: Option<Transition>, // `~name`: assemble in from a source cloud (vs a plain fade)
    deform: Option<Deform>, // `^name`: a persistent wobble while it's up
    deform_amp: Option<f32>, // `^name:amp`: scales the deform strength (None = 1.0)
    tint: Option<crate::scene::colorize::Tint>, // `tint:fry|rainbow|brand`: recolour the sampled splats
}

impl Prop {
    /// The object's source content — so the splat loader can collect its `splat:` filenames.
    pub(crate) fn content(&self) -> &PartContent {
        &self.content
    }

    /// One-line summary for the `MARTIN_VALIDATE` report.
    pub(crate) fn summary(&self) -> String {
        let mut s = format!(
            "{} @({:.1},{:.1},{:.1}) *{:.2}",
            self.content.label(),
            self.pos.x,
            self.pos.y,
            self.pos.z,
            self.scale
        );
        if let Some(t) = self.transition {
            s += &format!(" ~{t:?}");
        }
        if let Some(d) = self.deform {
            s += &format!(" ^{d:?}");
        }
        if self.appear >= 0.0 {
            s += &format!(" in@{:.1}s", self.appear);
        }
        s
    }

    /// The motion state carried on the spawned entity (shared by splat clouds + mesh props).
    /// `interpolate` = this object is a `GaussianInterpolate` (a `~transition`), so its `cs.time`
    /// is driven (the assemble) instead of an opacity fade-in. `field` is the scene-wide default
    /// deform (`MARTIN_DEFORM`) — a per-object `^deform` wins over it.
    fn anim(&self, base_rot: Quat, interpolate: bool, field: Option<Deform>) -> ComposeAnim {
        ComposeAnim {
            base_pos: self.pos,
            base_rot,
            base_scale: self.scale,
            spin: self.spin * (PI / 180.0),
            sway: self.sway * (PI / 180.0),
            bob: self.bob,
            drift: self.drift,
            appear: self.appear,
            out: self.out,
            fade: self.fade,
            interpolate,
            deform: self.deform.or(field).map(|d| {
                let (mode, amp, freq) = d.uniforms();
                (mode, amp * self.deform_amp.unwrap_or(1.0), freq)
            }),
            reveal: self.transition.and_then(|t| t.shader_uniforms()),
        }
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
    pub objects: Vec<Prop>,
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
    interpolate: bool, // a `~transition` object → drive cs.time (assemble), not opacity
    deform: Option<(u32, f32, f32)>, // `^name` deform uniforms (mode, amp, freq)
    reveal: Option<(u32, f32, u32)>, // per-particle transition shader (mode, softness, axis) — e.g.
                       // pen-write traces the letters in as it assembles (only while assembling, then off)
}

/// Parse `MARTIN_COMPOSE` (a file path or inline string). Each line: a `<source>` head (text/splat/
/// mesh/image) then placement tokens — `@x,y,z` position, `*scale`, `rot a,b,c`, `spin a,b,c`
/// (deg/s), `sway a,b,c` (deg), `bob amp`, `drift dx,dy,dz`, `in/out <anchor>` (section/bar/beat/s).
pub(crate) fn parse_compose(spec: &str, score: &score::Score) -> Vec<Prop> {
    let raw = std::fs::read_to_string(spec).unwrap_or_else(|_| spec.to_string());
    let kw = |t: &str| matches!(t, "rot" | "spin" | "sway" | "bob" | "drift" | "in" | "out");
    let mut out = Vec::new();
    // strip each line's `#` comment to end-of-line FIRST (so a `;` inside a comment can't split it
    // and leak the tail as a bogus object), then split into objects on `;`/newline.
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
        // pull the `~transition` + `^deform` + `tint:` tokens (position-independent), keep the rest.
        let mut transition = None;
        let mut deform = None;
        let mut deform_amp = None;
        let mut tint = None;
        let toks: Vec<&str> = s
            .split_whitespace()
            .filter(|t| {
                // a `~`/`^`/`tint:` token is always consumed; warn (don't leak) if it doesn't parse.
                if let Some(tr) = t.strip_prefix('~') {
                    match Transition::parse(tr) {
                        Some(x) => transition = Some(x),
                        None => eprintln!("compose: unknown transition '~{tr}' — ignored"),
                    }
                    return false;
                }
                if let Some(d) = t.strip_prefix('^') {
                    // `^name` or `^name:amp` — the optional amp scales this object's deform strength.
                    let (name, amp) = d.split_once(':').map_or((d, None), |(n, a)| (n, Some(a)));
                    match Deform::parse(name) {
                        Some(x) => {
                            deform = Some(x);
                            if let Some(a) = amp {
                                deform_amp = a.parse().ok();
                            }
                        }
                        None => eprintln!("compose: unknown deform '^{d}' — ignored"),
                    }
                    return false;
                }
                if let Some(tn) = t.strip_prefix("tint:") {
                    match crate::scene::colorize::Tint::parse(tn) {
                        Some(x) => tint = Some(x),
                        None => eprintln!("compose: unknown tint 'tint:{tn}' — ignored"),
                    }
                    return false;
                }
                true
            })
            .collect();
        // source = the leading tokens up to the first placement token (so `text:HELLO WORLD` works).
        let split = toks
            .iter()
            .position(|t| t.starts_with('@') || t.starts_with('*') || kw(t))
            .unwrap_or(toks.len());
        let Some(content) = parse_source(&toks[..split].join(" ")) else {
            eprintln!(
                "compose: unrecognized object '{}' — expected text:/svg:/image:/mesh:/glb:/model:/splat: — skipped",
                toks[..split].join(" ")
            );
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
        out.push(Prop {
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
            transition,
            deform,
            deform_amp,
            tint,
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
#[allow(clippy::too_many_arguments)]
pub(crate) fn build_composition(
    mut commands: Commands,
    mut assets: ResMut<Assets<PlanarGaussian3d>>,
    asset_server: Res<AssetServer>,
    comp: Option<ResMut<Composition>>,
    state: Option<Res<SeqState>>,
    seq: Option<Res<Sequence>>,
    root: Res<AssetRoot>,
    mut cam: Query<&mut OrbitCam>,
) {
    let (Some(mut comp), Some(state)) = (comp, state) else {
        return;
    };
    // A non-empty morph timeline is the "hero" track — it frames the camera; compose objects are
    // placed around it (tracks). Compose only frames the camera when it's the whole show.
    let hero = seq.map(|s| !s.parts.is_empty()).unwrap_or(false);
    if comp.built || comp.objects.is_empty() {
        return;
    }
    if state.loads.iter().any(|h| assets.get(h).is_none()) {
        return; // wait for every referenced splat
    }
    let base = cloud_base_rotation();
    // MARTIN_DEFORM = a scene-wide FIELD: it wobbles every object on the stage (a per-object
    // `^deform` overrides it), the same default the morph timeline uses — wind over the whole scene.
    let field = std::env::var("MARTIN_DEFORM")
        .ok()
        .and_then(|s| Deform::parse(&s));
    // cap each object's splats so a stage of big splats stays performant on the iGPU.
    let count = crate::envvar::or("MARTIN_MORPH_COUNT", 120_000);
    let mut placed: Vec<(Vec3, f32)> = Vec::new(); // (centre, radius) per object, for framing
    let mut any_model = false;
    for obj in &comp.objects {
        // A real glTF mesh prop: rendered as PBR geometry alongside the splats (no flip — glTF is
        // Y-up native; the splats are flipped to match). It shares the camera + depth buffer, so it
        // composites with the splat clouds. Spawned, not sampled to gaussians.
        if let PartContent::Model(name) = &obj.content {
            let rot = Quat::from_euler(
                EulerRot::XYZ,
                obj.rot.x.to_radians(),
                obj.rot.y.to_radians(),
                obj.rot.z.to_radians(),
            );
            commands.spawn((
                SceneRoot(asset_server.load(GltfAssetLabel::Scene(0).from_asset(name.clone()))),
                Transform {
                    translation: obj.pos,
                    rotation: rot,
                    scale: Vec3::splat(obj.scale),
                },
                obj.anim(rot, false, field),
            ));
            placed.push((obj.pos, 1.0)); // rough radius (the mesh size is async); tune with MARTIN_ZOOM
            any_model = true;
            continue;
        }
        // `text:~pen-write` builds SINGLE-STROKE handwriting gaussians (thin centerline strokes) so
        // the mode-7 reveal traces real handwriting — same as the reel. Plain text would just get a
        // filled-outline trace that doesn't read as writing. Other content goes through part_gaussians.
        let mut raw = match (&obj.content, obj.transition) {
            (PartContent::Text(s), Some(Transition::PenWrite)) => {
                let pw_step = crate::envvar::or("MARTIN_PW_STEP", 0.5_f32);
                let pw_splat = crate::envvar::or("MARTIN_PW_SPLAT", 0.006_f32);
                crate::text::build_text_penwrite_gaussians(
                    s,
                    crate::text::TEXT_RGB,
                    3.0,
                    pw_step,
                    pw_splat,
                )
            }
            _ => part_gaussians(&obj.content, &state, &assets, &root.0),
        };
        if raw.is_empty() {
            continue;
        }
        crate::morph::normalize_to(&mut raw, NORMALIZE_EXTENT); // centre + ~2 units across
        let mut shaped = resample_morton(raw, count);
        // `tint:` recolours the sampled cloud (e.g. a deep-fried bitterbal) before it's frozen into
        // the shape + its transition source — so both the held look and the assemble-in are tinted.
        if let Some(tint) = obj.tint {
            crate::scene::colorize::apply(&mut shaped, tint);
        }
        let rot = Quat::from_euler(
            EulerRot::XYZ,
            obj.rot.x.to_radians(),
            obj.rot.y.to_radians(),
            obj.rot.z.to_radians(),
        ) * base;
        let cs = CloudSettings {
            sort_mode: SortMode::Radix,
            time: 0.0,
            time_start: 0.0,
            time_stop: 1.0,
            bulge: 0.0,
            global_opacity: 0.0, // animate_composition fades it in (or holds it for a transition)
            ..default()
        };
        let tf = Transform {
            translation: obj.pos,
            rotation: rot,
            scale: Vec3::splat(obj.scale),
        };
        // A `~transition` object ASSEMBLES from a source cloud → a GaussianInterpolate (its cs.time
        // is driven by animate_composition). Without one it's a PLAIN static cloud (a fade-in) — no
        // per-frame GPU blend. Plain `PlanarGaussian3dHandle` renders directly; `calculate_bounds`
        // gives both kinds an Aabb (NO NoFrustumCulling — that would skip the Aabb → black screen).
        if let Some(tr) = obj.transition {
            let source =
                source_cloud(tr, &shaped, NORMALIZE_EXTENT * 0.5).unwrap_or_else(|| shaped.clone());
            commands.spawn((
                GaussianInterpolate::<Gaussian3d> {
                    lhs: PlanarGaussian3dHandle(assets.add(PlanarGaussian3d::from(source))),
                    rhs: PlanarGaussian3dHandle(assets.add(PlanarGaussian3d::from(shaped))),
                },
                Visibility::Visible,
                cs,
                tf,
                obj.anim(rot, true, field),
            ));
        } else {
            commands.spawn((
                PlanarGaussian3dHandle(assets.add(PlanarGaussian3d::from(shaped))),
                Visibility::Visible,
                cs,
                tf,
                obj.anim(rot, false, field),
            ));
        }
        placed.push((obj.pos, NORMALIZE_EXTENT * 0.5 * obj.scale));
    }
    // Mesh props need lighting (the splats don't); a key + a soft fill so no side goes black.
    if any_model {
        commands.spawn((
            DirectionalLight {
                illuminance: 9000.0,
                shadows_enabled: false,
                ..default()
            },
            Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, 0.6, 0.0)),
        ));
        commands.spawn((
            DirectionalLight {
                illuminance: 3500.0,
                shadows_enabled: false,
                ..default()
            },
            Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, 0.7, -0.8, 0.0)),
        ));
    }
    comp.built = true;
    // a hero morph track owns the camera; with one present, don't re-frame on the compose objects.
    if placed.is_empty() || hero {
        return;
    }
    let center = placed.iter().map(|(p, _)| *p).sum::<Vec3>() / placed.len() as f32;
    let radius = placed
        .iter()
        .map(|(p, r)| (*p - center).length() + r)
        .fold(0.1_f32, f32::max);
    let zoom = crate::envvar::or("MARTIN_ZOOM", 1.0_f32);
    let zoom = if zoom > 0.0 { zoom } else { 1.0 }; // a non-positive zoom is meaningless → default
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
    // CloudSettings is optional: splat objects have it (opacity fade/flare), mesh props don't (they
    // still spin/bob/drift via Transform).
    mut q: Query<(&ComposeAnim, &mut Transform, Option<&mut CloudSettings>)>,
) {
    let t = clock.t;
    let k = beat.intensity;
    for (a, mut tf, cs) in &mut q {
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
        // in/out visibility (0..1): splats fade it via opacity; mesh props (no CloudSettings) have
        // no opacity, so they DISSOLVE by SCALE instead — grow in, shrink out. That gives a clean
        // mesh→splat cross-dissolve (a mesh shrinks away as a splat fades/grows in at the same spot).
        let fin = if a.appear < 0.0 {
            1.0
        } else {
            ((t - a.appear) / a.fade.max(1e-3)).clamp(0.0, 1.0)
        };
        let fout = if a.out < f32::MAX {
            ((a.out + a.fade - t) / a.fade.max(1e-3)).clamp(0.0, 1.0)
        } else {
            1.0
        };
        let vis = fin.min(fout);
        let has_cs = cs.is_some();
        let scale_vis = if has_cs { 1.0 } else { vis }; // meshes carry the fade in their scale
        // kick thumps the scale (bulge is a no-op on a static cloud, so scale carries it too).
        tf.scale = Vec3::splat(a.base_scale * scale_vis * (1.0 + beat.kick * 0.06 * k));
        if let Some(mut cs) = cs {
            // a `~transition` object assembles via the morph (cs.time) — its IN-fade is the morph, so
            // only the OUT fade touches opacity. BUT it must stay HIDDEN until its `in` cue, else its
            // source cloud (cs.time=0) renders from t=0 (a bitterbal floating through the intro).
            let started = a.appear < 0.0 || t >= a.appear;
            let op = if a.interpolate {
                if started { fout } else { 0.0 }
            } else {
                vis
            };
            cs.global_opacity = op * (1.0 + (beat.snare * 0.4 + beat.hat * 0.12) * k);
            if a.interpolate {
                let f = ((t - a.appear.max(0.0)) / COMPOSE_MORPH).clamp(0.0, 1.0);
                cs.time = f * f * (3.0 - 2.0 * f); // eased assemble
                // run the per-particle reveal shader WHILE assembling (pen-write traces the letters
                // in), then switch it off so the held cloud renders plain + sort-safe.
                let (mode, soft, axis) = if f < 1.0 {
                    a.reveal.unwrap_or((0, 0.0, 0))
                } else {
                    (0, 0.0, 0)
                };
                cs.transition_mode = mode;
                cs.transition_softness = soft;
                cs.transition_axis = axis;
            }
            if let Some((mode, amp, freq)) = a.deform {
                cs.deform_mode = mode;
                cs.deform_amp = amp * (1.0 + (beat.kick * 0.6 + beat.snare * 0.3) * k);
                cs.deform_freq = freq;
                cs.deform_time = t * 2.0;
            }
        }
    }
}

/// Slowly orbit the camera around the stage (the "flow") — additive with the live arrow keys.
pub(crate) fn compose_camera(
    comp: Option<Res<Composition>>,
    seq: Option<Res<Sequence>>,
    rec: Res<RecordState>,
    time: Res<Time>,
    mut cam: Query<&mut OrbitCam>,
) {
    // when a morph hero track is present it owns the camera (its own sway/flypath) — don't add a drift.
    if seq.map(|s| !s.parts.is_empty()).unwrap_or(false) {
        return;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn objs(spec: &str) -> Vec<Prop> {
        parse_compose(spec, &score::Score::builtin())
    }

    #[test]
    fn parse_compose_reads_placement_and_transition() {
        let o = objs("mesh:bitterbal.obj @1.5,-0.3,0 *0.5 spin 0,40,0 ~drop");
        assert_eq!(o.len(), 1);
        let s = o[0].summary();
        assert!(s.contains("mesh bitterbal.obj"), "{s}");
        assert!(s.contains("@(1.5,-0.3,0.0)"), "{s}");
        assert!(s.contains("*0.50"), "{s}");
        assert!(s.contains("~Drop"), "{s}");
    }

    #[test]
    fn multi_word_head_before_placement_token() {
        // `text:HELLO WORLD` must keep both words as the head (split at the first @/*/keyword).
        let o = objs("text:HELLO WORLD @0,0,0 *1");
        assert_eq!(o.len(), 1);
        assert!(
            o[0].summary().contains("text \"HELLO WORLD\""),
            "{}",
            o[0].summary()
        );
    }

    #[test]
    fn unknown_object_is_skipped_and_bad_modifiers_dont_leak() {
        let o = objs("text:OK @0,0,0 ~fade; bogus:x @1,0,0; text:B @0,0,0 ~explod");
        assert_eq!(o.len(), 2); // bogus:x dropped
        assert!(o[0].summary().contains("~Fade"));
        assert!(!o[1].summary().contains('~')); // ~explod didn't parse, not leaked
    }

    #[test]
    fn comment_with_semicolon_does_not_resurrect_an_object() {
        // same regression guard as the seq parser.
        let o = objs("text:A @0,0,0  # note; with a ; and ~fade\ntext:B @1,0,0");
        assert_eq!(o.len(), 2);
        assert!(!o[0].summary().contains('~'));
    }

    #[test]
    fn deform_token_parses_on_a_compose_object() {
        let o = objs("text:WOBBLE @0,0,0 ^wave");
        assert!(o[0].summary().contains("^Wave"), "{}", o[0].summary());
    }
}
