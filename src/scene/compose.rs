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
use crate::scene::content::{PartContent, parse_source, part_gaussians, sample_content_owned};
use crate::scene::effects::{Deform, Ease, Entrance, source_cloud};
use crate::scene::sequence::{SeqState, Sequence};
use crate::scene::{AssetRoot, NORMALIZE_EXTENT, SeqClock, cloud_base_rotation};
use crate::score;

const COMPOSE_MORPH: f32 = 3.6; // how long a `~entrance` compose object takes to assemble in (s) —
// long enough that a pen-write breathes as the letters are drawn in stroke by stroke

/// A per-object motion PATH — independent travel layered on top of `@pos + drift`, so each object can
/// "do its own thing" (orbit, or weave a Lissajous figure) instead of holding still + only beat-pulsing.
/// `path:orbit:r,speed` / `path:orbit:rx,rz,speed` / `path:liss:ax,ay,az,fx,fy,fz[,speed]`.
#[derive(Clone, Copy)]
pub(crate) enum PathMotion {
    Orbit { rx: f32, rz: f32, speed: f32 }, // ellipse in the XZ plane around @pos
    Lissajous { amp: Vec3, freq: Vec3, speed: f32 }, // 3D Lissajous figure around @pos
}

impl PathMotion {
    /// Position offset from `@pos` at show-time `t` (seconds). Phase-shifts Z by π/2 so a Lissajous
    /// with equal x/z is a circle, not a diagonal line.
    fn offset(self, t: f32) -> Vec3 {
        match self {
            PathMotion::Orbit { rx, rz, speed } => {
                Vec3::new(rx * (speed * t).cos(), 0.0, rz * (speed * t).sin())
            }
            PathMotion::Lissajous { amp, freq, speed } => Vec3::new(
                amp.x * (freq.x * speed * t).sin(),
                amp.y * (freq.y * speed * t).sin(),
                amp.z * (freq.z * speed * t + std::f32::consts::FRAC_PI_2).sin(),
            ),
        }
    }

    fn parse(s: &str) -> Option<Self> {
        let (kind, rest) = s.split_once(':')?;
        let n: Vec<f32> = rest
            .split(',')
            .filter_map(|x| x.trim().parse().ok())
            .collect();
        match kind {
            "orbit" | "circle" => match n.len() {
                2 => Some(PathMotion::Orbit {
                    rx: n[0],
                    rz: n[0],
                    speed: n[1],
                }),
                3 => Some(PathMotion::Orbit {
                    rx: n[0],
                    rz: n[1],
                    speed: n[2],
                }),
                _ => None,
            },
            "liss" | "lissajous" => (n.len() >= 6).then(|| PathMotion::Lissajous {
                amp: Vec3::new(n[0], n[1], n[2]),
                freq: Vec3::new(n[3], n[4], n[5]),
                speed: *n.get(6).unwrap_or(&1.0),
            }),
            _ => None,
        }
    }
}

/// `travel:x,y,z[@anchor[,dur]]` — ease the object from its `@pos` to a fixed target over a window,
/// then HOLD at the target. Unlike `drift` (linear, never stops) or `path:` (oscillatory), this is a
/// one-shot A→B move (the horse walk-in). Start = its `in` cue (or a given anchor); over `dur` s.
#[derive(Clone, Copy)]
pub(crate) struct Travel {
    target: Vec3,
    start: f32, // show-clock seconds the move begins
    dur: f32,   // move duration (s); > 0 guaranteed at construction
}

impl Travel {
    /// Eased offset to ADD to `base_pos` at show-time `t`: 0 before `start`, eases base→target across
    /// `[start, start+dur]`, then holds. `ease` shapes the curve (reuses the object's assemble Ease).
    fn offset(self, base_pos: Vec3, ease: Ease, t: f32) -> Vec3 {
        let f = ((t - self.start) / self.dur).clamp(0.0, 1.0);
        (self.target - base_pos) * ease.apply(f)
    }
}

/// One object placed on the composition stage: a source + where it sits + how it moves.
#[derive(Clone)]
pub(crate) struct Prop {
    content: PartContent,
    pos: Vec3,
    scale: Vec3, // per-axis scale (`*s` → uniform, `*sx,sy,sz` → stretched, e.g. a wide ridge)
    rot: Vec3,   // static orientation, euler degrees
    spin: Vec3,  // auto-rotation, degrees/sec
    sway: Vec3, // oscillating rotation amplitude, degrees (swings front-on; for hollow-back splats)
    bob: f32,   // vertical bob amplitude (units)
    drift: Vec3, // translation velocity (units/sec)
    appear: f32, // fade-in start (s on the show clock)
    out: f32,   // fade-out start (s); f32::MAX = stays to the end
    fade: f32,  // fade in/out duration (s)
    entrance: Option<Entrance>, // `~name`: assemble in from a source cloud (vs a plain fade)
    deform: Option<Deform>, // `^name`: a persistent wobble while it's up
    deform_amp: Option<f32>, // `^name:amp`: scales the deform strength (None = 1.0)
    tint: Option<crate::scene::colorize::Tint>, // `tint:fry|rainbow|brand`: recolour the sampled splats
    ease: Ease, // `ease:name`: shapes the assemble curve (default Smoothstep = unchanged)
    field: Option<usize>, // `field:N`: scatter into N seeded copies (a swarm/field of this object)
    count: Option<usize>, // `count:N`: per-object splat count (density), overrides the scene MORPH_COUNT
    path: Option<PathMotion>, // `path:orbit/liss:…`: independent travel path (each object does its own thing)
    alpha: Option<f32>, // `alpha:V`: per-object translucency (0..1) baked into the cloud's splat opacity
    travel: Option<Travel>, // `travel:x,y,z[@anchor[,dur]]`: ease @pos→target over a window, then hold
    font: Option<String>, // `font:<name>`: pick a `text:` font (e.g. `defeest`); None = default font.ttf
    disk_scale: Option<f32>, // `dscale:V`: per-object splat-disk size — a LOCAL MARTIN_SPLAT_SCALE. <1
    // shrinks just this object's disks (cut a full-screen backdrop's overdraw without thinning props);
    // multiplies the global MARTIN_SPLAT_SCALE.
    disk: Option<f32>, // `disk:V`: MESH-sampling disk size (the size of each splat as a mesh is sampled
    // into gaussians) — a DIFFERENT knob from `dscale:` (a render-time multiplier). Threaded into
    // sample_content; was silently dropped, so a compose mesh's `disk:`/`aniso:` had no effect.
    aniso: Option<f32>, // `aniso:V`: mesh-sampling anisotropy (stretch the sampled splats).
    additive: Option<bool>, // `additive:0|1`: per-object emissive glow blend; None = MARTIN_ADDITIVE
}

impl Prop {
    /// The object's source content — so the splat loader can collect its `splat:` filenames.
    pub(crate) fn content(&self) -> &PartContent {
        &self.content
    }

    /// One-line summary for the `MARTIN_VALIDATE` report.
    pub(crate) fn summary(&self) -> String {
        let sc = if self.scale.x == self.scale.y && self.scale.y == self.scale.z {
            format!("*{:.2}", self.scale.x)
        } else {
            format!(
                "*{:.2},{:.2},{:.2}",
                self.scale.x, self.scale.y, self.scale.z
            )
        };
        let mut s = format!(
            "{} @({:.1},{:.1},{:.1}) {sc}",
            self.content.label(),
            self.pos.x,
            self.pos.y,
            self.pos.z,
        );
        if let Some(t) = self.entrance {
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

    /// Estimated peak resident gaussians for this prop: its cloud + (for a `~entrance`) its source
    /// cloud → ×2. `scene_default` is the count for a prop with no `count:` (MARTIN_MORPH_COUNT).
    /// Mirrors the build-time sample math so `--validate` can flag OOM risk GPU-free (count-bound).
    pub(crate) fn resident_estimate(&self, scene_default: usize) -> usize {
        let n = crate::scene::cap_count(
            "compose",
            ((self.count.unwrap_or(scene_default) as f32 * crate::scene::count_scale()).round()
                as usize)
                .max(64),
        );
        // a SPATIAL `~entrance` holds a source cloud too (×2); `~fade` is the plain opacity path → ×1.
        let doubles = matches!(self.entrance, Some(t) if t != Entrance::Fade);
        n * if doubles { 2 } else { 1 }
    }

    /// The motion state carried on the spawned entity (shared by splat clouds + mesh props).
    /// `interpolate` = this object is a `GaussianInterpolate` (a `~entrance`), so its `cs.time`
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
            reveal: self.entrance.and_then(|t| t.shader_uniforms()),
            ease: self.ease,
            path: self.path,
            travel: self.travel,
        }
    }
}

/// How long to record a composition stage: enough for every object to have appeared (and any
/// fade-out to finish), plus a tail. The recorder uses this since a compose stage has no morph
/// timeline to derive an end from. A free fn (not just `Composition::record_secs`) so `--validate`
/// can print it from a bare `&[Prop]` too, without needing a built `Composition` — `record.sh` parses
/// it (alongside the sequence line) to catch a reel/compose that outlasts the score.
pub(crate) fn compose_record_secs(objects: &[Prop]) -> f32 {
    let mut end = 0.0_f32;
    for o in objects {
        end = end.max(o.appear.max(0.0));
        if o.out < f32::MAX {
            end = end.max(o.out + o.fade);
        }
    }
    (end + 8.0).max(12.0)
}

impl Composition {
    /// See [`compose_record_secs`].
    pub(crate) fn record_secs(&self) -> f32 {
        compose_record_secs(&self.objects)
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
    base_scale: Vec3,
    spin: Vec3, // rad/sec
    sway: Vec3, // rad amplitude, oscillating
    bob: f32,
    drift: Vec3,
    appear: f32,
    out: f32,
    fade: f32,
    interpolate: bool, // a `~entrance` object → drive cs.time (assemble), not opacity
    deform: Option<(u32, f32, f32)>, // `^name` deform uniforms (mode, amp, freq)
    reveal: Option<(u32, f32, u32)>, // the entrance's per-particle reveal shader (mode/softness/axis) —
    // pen-write traces the letters in as it assembles (only while assembling, then off)
    ease: Ease,               // shapes the assemble curve (`ease:`)
    path: Option<PathMotion>, // independent travel path (`path:orbit/liss:…`)
    travel: Option<Travel>,   // one-shot eased A→B move, then hold (`travel:`)
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
        // pull the `~entrance` + `^deform` + `tint:` tokens (position-independent), keep the rest.
        let mut entrance = None;
        let mut deform = None;
        let mut deform_amp = None;
        let mut tint = None;
        let mut ease = Ease::Smoothstep;
        let mut field = None;
        let mut count_override = None;
        let mut path = None;
        let mut alpha = None;
        let mut font = None;
        let mut disk_scale = None;
        let mut disk = None;
        let mut aniso = None;
        let mut additive = None;
        // travel: parsed raw here (needs `score` + the object's `in` cue, both resolved after the loop)
        let mut travel_raw: Option<(Vec3, Option<String>, Option<f32>)> = None;
        let toks: Vec<&str> = s
            .split_whitespace()
            .filter(|t| {
                // `field:N` — scatter this object into N seeded, randomly-rotated copies (a "swarm"/
                // field of it), like the reel's `flock:`. Consumed here so it never leaks into the head.
                if let Some(n) = t.strip_prefix("field:") {
                    match n.parse() {
                        Ok(c) => field = Some(c),
                        Err(_) => eprintln!("compose: bad 'field:{n}' (need an integer) — ignored"),
                    }
                    return false;
                }
                // `count:N` — per-object splat COUNT (density), overriding the scene-wide MORPH_COUNT.
                // Density is a property of the object: a tent is denser than a tree, a tree denser than
                // fire. (Translucency = the baked per-shape opacity; this is the other half.)
                if let Some(n) = t.strip_prefix("count:") {
                    match n.parse() {
                        Ok(c) => count_override = Some(c),
                        Err(_) => eprintln!("compose: bad 'count:{n}' (need an integer) — ignored"),
                    }
                    return false;
                }
                // `font:<name>` — pick a `text:` font (e.g. `defeest`); default = bold font.ttf.
                if let Some(f) = t.strip_prefix("font:") {
                    font = Some(f.to_string());
                    return false;
                }
                // `alpha:V` — per-object translucency (0..1), baked into the cloud's splat opacity at
                // build (a glass beer mug, a ghost, a haze). The scene-wide fade-in animates on top.
                if let Some(n) = t.strip_prefix("alpha:") {
                    match finite_f32(n) {
                        Some(a) => alpha = Some(a.clamp(0.0, 1.0)),
                        None => eprintln!("compose: bad 'alpha:{n}' (need a finite 0..1) — ignored"),
                    }
                    return false;
                }
                // `dscale:V` — per-object splat-DISK size (a local MARTIN_SPLAT_SCALE). <1 shrinks just
                // this object's disks → kills a full-screen backdrop's overdraw without thinning the props.
                if let Some(n) = t.strip_prefix("dscale:") {
                    match finite_f32(n) {
                        Some(d) => disk_scale = Some(d.max(0.0)),
                        None => eprintln!("compose: bad 'dscale:{n}' (need a finite float) — ignored"),
                    }
                    return false;
                }
                // `disk:V` / `aniso:V` — MESH-sampling knobs (splat size + anisotropy as a mesh is
                // sampled into gaussians), passed into sample_content. Were silently dropped before.
                if let Some(n) = t.strip_prefix("disk:") {
                    match finite_f32(n) {
                        Some(d) => disk = Some(d.max(0.0)),
                        None => eprintln!("compose: bad 'disk:{n}' (need a finite float) — ignored"),
                    }
                    return false;
                }
                if let Some(n) = t.strip_prefix("aniso:") {
                    match finite_f32(n) {
                        Some(d) => aniso = Some(d.max(0.0)),
                        None => eprintln!("compose: bad 'aniso:{n}' (need a finite float) — ignored"),
                    }
                    return false;
                }
                // `additive:0|1` — per-object emissive glow blend override (else MARTIN_ADDITIVE).
                if let Some(n) = t.strip_prefix("additive:") {
                    match finite_f32(n) {
                        Some(v) => additive = Some(v != 0.0),
                        None => eprintln!("compose: bad 'additive:{n}' (need 0 or 1) — ignored"),
                    }
                    return false;
                }
                // `path:orbit:…` / `path:liss:…` — an independent motion path (each object does its own
                // thing: orbits, weaves a Lissajous figure) layered on @pos + drift.
                if let Some(p) = t.strip_prefix("path:") {
                    match PathMotion::parse(p) {
                        Some(pm) => path = Some(pm),
                        None => eprintln!("compose: bad 'path:{p}' (orbit:r,speed | liss:ax,ay,az,fx,fy,fz[,s]) — ignored"),
                    }
                    return false;
                }
                // `travel:x,y,z[@anchor[,dur]]` — one-shot eased move from @pos to a target, then hold
                // (the horse walk-in). Resolved to seconds after the loop (needs the `in` cue + score).
                if let Some(v) = t.strip_prefix("travel:") {
                    let (xyz, spec) = v.split_once('@').map_or((v, None), |(a, b)| (a, Some(b)));
                    let (anchor, dur) = match spec {
                        Some(s) => {
                            let (a, d) = s.split_once(',').map_or((s, None), |(a, d)| (a, Some(d)));
                            (
                                (!a.is_empty()).then(|| a.to_string()),
                                d.and_then(finite_f32),
                            )
                        }
                        None => (None, None),
                    };
                    travel_raw = Some((vec3_csv(xyz), anchor, dur));
                    return false;
                }
                // the shared `~entrance` / `^deform[:amp]` / `tint:` modifiers (see effects.rs); a
                // recognised sigil is always consumed (warn on a bad value, don't leak it).
                if let Some(res) = crate::scene::effects::parse_fx_modifier(t) {
                    use crate::scene::effects::FxMod;
                    match res {
                        Ok(FxMod::Entrance(x)) => entrance = Some(x),
                        Ok(FxMod::Deform(d, a, _)) => {
                            // `@morph` gating is reel-only (compose objects don't morph) → ignore it.
                            deform = Some(d);
                            deform_amp = a;
                        }
                        Ok(FxMod::Tint(x)) => tint = Some(x),
                        Ok(FxMod::Ease(e)) => ease = e,
                        Err(w) => eprintln!("compose: {w} — ignored"),
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
        let (mut pos, mut scale, mut rot) = (Vec3::ZERO, Vec3::ONE, Vec3::ZERO);
        let (mut spin, mut sway, mut bob, mut drift) =
            (Vec3::ZERO, Vec3::ZERO, 0.0_f32, Vec3::ZERO);
        // appear < 0 = no fade-in (visible from the start); `in <anchor>` sets it to a real time.
        let (mut appear, mut out_t, fade) = (-1.0_f32, f32::MAX, 0.8_f32);
        let mut i = 0;
        while i < rest.len() {
            let t = rest[i];
            if let Some(v) = t.strip_prefix('@') {
                // `@x,y,z` (tight) OR a stray-space `@ x,y,z` — be forgiving: a bare `@` token used to
                // strip to "" → pos silently collapsed to the ORIGIN (and ate the next token), stacking
                // every object dead-centre. Treat the following token as the coords instead.
                let (v, step) = if v.is_empty() {
                    (rest.get(i + 1).copied().unwrap_or(""), 2)
                } else {
                    (v, 1)
                };
                pos = vec3_csv(v);
                i += step;
            } else if let Some(v) = t.strip_prefix('*') {
                // `*s` = uniform; `*sx,sy,sz` = per-axis stretch (e.g. a wide, low mountain ridge).
                // Forgiving of a stray-space `* s` the same way `@` is.
                let (v, step) = if v.is_empty() {
                    (rest.get(i + 1).copied().unwrap_or(""), 2)
                } else {
                    (v, 1)
                };
                scale = if v.contains(',') {
                    vec3_csv(v)
                } else {
                    Vec3::splat(v.parse().unwrap_or(1.0))
                };
                i += step;
            } else if kw(t) {
                // a space-separated keyword that takes the NEXT token as its value (`rot 0,0,0`).
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
            } else {
                // An unrecognized stray token (a typo, or a token only valid in `[reel]` like `beat:`).
                // Skip just IT — advancing by 2 here would swallow the following token, and if that was
                // `in`/`out` the next `@anchor` got mis-read as an `@pos` → the object snapped to the
                // ORIGIN, stacking dead-centre. Skip-by-one keeps stray tokens harmless.
                i += 1;
            }
        }
        // resolve `travel:` now that @pos (`pos`) and the `in` cue (`appear`) are final. Start = the
        // given `@anchor`, else the object's `in` time, else 0. Duration defaults to one bar.
        let travel = travel_raw.map(|(target, anchor, dur)| Travel {
            target,
            start: anchor
                .and_then(|a| score.anchor_seconds(&a))
                .unwrap_or_else(|| appear.max(0.0)),
            dur: dur.filter(|d| *d > 0.0).unwrap_or_else(|| score.bar()),
        });
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
            entrance,
            deform,
            deform_amp,
            tint,
            ease,
            field,
            count: count_override,
            path,
            alpha,
            travel,
            font,
            disk_scale,
            disk,
            aniso,
            additive,
        });
    }
    out
}

/// Parse a scalar token strictly: reject `nan`/`inf`/`-inf` (Rust's `f32::from_str` accepts these
/// literals, so a bare `.parse::<f32>()` lets them through) BEFORE the caller's own `.clamp()`/`.max()`
/// — `f32::clamp` does not sanitize NaN (`NaN.clamp(0,1)` is still NaN), and `inf as u32` elsewhere
/// saturates to `u32::MAX` (a duration this large would wedge a frame-count loop). System-boundary
/// check: every free-form `key:value` scalar in a `.show`/compose token should parse through this.
fn finite_f32(s: &str) -> Option<f32> {
    s.trim().parse::<f32>().ok().filter(|f| f.is_finite())
}

/// Parse `x,y,z` POSITION-STRICTLY and reject non-finite. The old `filter_map(parse.ok())` dropped a bad
/// component and shifted the rest (`@1,O,3` → `(1,3,0)` — object silently mis-placed), and let `nan`/`1e40`
/// through into the Transform → a NaN camera framing blacks the whole render. Now each slot is parsed by
/// INDEX; a missing/unparseable/non-finite component keeps its 0.0 default and warns (no shift, no poison).
fn vec3_csv(s: &str) -> Vec3 {
    let mut a = [0.0_f32; 3];
    for (i, part) in s.split(',').take(3).enumerate() {
        match part.trim().parse::<f32>() {
            Ok(f) if f.is_finite() => a[i] = f,
            Ok(_) => eprintln!(
                "compose: non-finite vec component '{}' in '{s}' — using 0",
                part.trim()
            ),
            Err(_) if !part.trim().is_empty() => {
                eprintln!(
                    "compose: bad vec component '{}' in '{s}' — using 0",
                    part.trim()
                )
            }
            Err(_) => {}
        }
    }
    Vec3::from_array(a)
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
    // Wait until every referenced splat has SETTLED (loaded OR failed) — not "is_none()", which blocks
    // forever on a `Failed` handle (a missing/corrupt/typo'd .ply hung the whole stage indefinitely). A
    // failed load falls through; that object samples 0 gaussians and is simply absent.
    {
        let mut failed = Vec::new();
        for (h, name) in state.loads.iter().zip(&state.load_names) {
            match asset_server.load_state(h) {
                bevy::asset::LoadState::Loaded => {}
                bevy::asset::LoadState::Failed(_) => failed.push(name.as_str()),
                _ => return,
            }
        }
        if !failed.is_empty() {
            warn!(
                "compose: {} splat(s) failed to load: {failed:?} — stage renders without them",
                failed.len()
            );
        }
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
    let mut resident = 0usize; // running estimate of peak resident gaussians (for the OOM warning)

    // #7 — the reel got per-object parallelism (build.rs); this stage sampled one object at a time.
    // A Model/GlMesh prop is spawned directly (async asset load, no CPU sampling — stays serial, cheap,
    // main-thread-only Bevy API). Everything else's CPU-heavy chain (sample → normalize → resample/
    // cluster → tint → alpha) has NO cross-object dependency (unlike the reel's shared `scene_norm`,
    // compose normalizes every object to the same constant `NORMALIZE_EXTENT` independently) — so it
    // can run fully in parallel, producing an owned `shaped: Vec<Gaussian3d>` per object. Only the
    // final `assets.add`/`commands.spawn` (main-thread-only) stays serial, in a later pass.
    //
    // Splat-content pre-extraction (the only step needing `&Assets`) happens HERE, serially, mirroring
    // build.rs's `presampled` — so the parallel closure below is asset-free (`sample_content_owned`).
    struct Prepared {
        obj_count: usize,
        sample_n: usize,
        presample: Option<Vec<Gaussian3d>>,
    }
    let mut prepared: Vec<Option<Prepared>> = Vec::with_capacity(comp.objects.len());
    for obj in &comp.objects {
        if matches!(obj.content, PartContent::Model(_) | PartContent::GlMesh(_)) {
            prepared.push(None);
            continue;
        }
        // `count:N` overrides the scene-wide density for THIS object; `field:N` splits it across N swarm
        // copies. Sample a MESH at this final per-object count (passed into sample_content_owned) so its
        // disk size matches the eventual density — sampling high then downsampling left mesh disks too
        // small for the thinned spacing → holes (most visible on the big props, e.g. horses, at 1080p).
        // MARTIN_COUNT_SCALE thins EVERY prop (count: or default) — lets QUALITY scale a count-bound
        // compose stage, which a per-object `count:` would otherwise pin past MARTIN_MORPH_COUNT.
        let obj_count = crate::scene::cap_count(
            "compose",
            ((obj.count.unwrap_or(count) as f32 * crate::scene::count_scale()).round() as usize)
                .max(64),
        );
        // Peak resident: its shape cloud + (for a SPATIAL `~entrance`) its source cloud → ×2. `~fade`
        // is the plain opacity path now (no source cloud), so it counts ×1.
        let doubles = matches!(obj.entrance, Some(t) if t != Entrance::Fade);
        resident += obj_count * if doubles { 2 } else { 1 };
        let sample_n = match obj.field {
            Some(n) => (obj_count / n.max(1)).max(2_000),
            None => obj_count,
        };
        let presample = matches!(obj.content, PartContent::Splats(_)).then(|| {
            part_gaussians(
                &obj.content,
                &state,
                &assets,
                &root.0,
                obj.disk,
                obj.aniso,
                Some(sample_n),
                obj.font.as_deref(),
            )
        });
        prepared.push(Some(Prepared {
            obj_count,
            sample_n,
            presample,
        }));
    }

    let shaped_all: Vec<Option<Vec<Gaussian3d>>> = {
        use rayon::prelude::*;
        comp.objects
            .par_iter()
            .zip(prepared.par_iter_mut())
            .map(|(obj, prep)| {
                let prep = prep.as_mut()?;
                let mut raw = sample_content_owned(
                    &obj.content,
                    obj.entrance,
                    prep.presample.take(),
                    &root.0,
                    obj.disk,
                    obj.aniso,
                    Some(prep.sample_n),
                    obj.font.as_deref(),
                );
                if raw.is_empty() {
                    return None;
                }
                crate::morph::normalize_to(&mut raw, NORMALIZE_EXTENT); // centre + ~2 units across
                // `field:N` scatters the object into N seeded copies (a swarm), sized to frame as one —
                // the reel's `flock:` for the stage. Downsample per copy to keep the total near budget.
                let mut shaped = match obj.field {
                    Some(n) => {
                        let per = (prep.obj_count / n.max(1)).max(2_000);
                        crate::morph::cluster_of(&resample_morton(raw, per), n)
                    }
                    None => resample_morton(raw, prep.obj_count),
                };
                // `tint:` recolours the sampled cloud (e.g. a deep-fried bitterbal) before it's frozen
                // into the shape + its entrance source — so both the held look + assemble-in are tinted.
                if let Some(tint) = obj.tint {
                    crate::scene::colorize::apply(&mut shaped, tint);
                }
                // `alpha:V` — bake per-object translucency into splat opacity (a glass mug, a ghost).
                if let Some(a) = obj.alpha {
                    for g in &mut shaped {
                        let sc = g.scale_opacity.scale;
                        let op = g.scale_opacity.opacity;
                        g.scale_opacity = [sc[0], sc[1], sc[2], op * a].into();
                    }
                }
                Some(shaped)
            })
            .collect()
    };

    for (obj, shaped) in comp.objects.iter().zip(shaped_all) {
        // A real glTF mesh prop: rendered as PBR geometry alongside the splats (no flip — glTF is
        // Y-up native; the splats are flipped to match). It shares the camera + depth buffer, so it
        // composites with the splat clouds. Spawned, not sampled to gaussians. Both `model:` and `glb:`
        // (GlMesh) render here as a rigid lit prop — the dissolve-into-own-splats path is reel-only.
        if let PartContent::Model(name) | PartContent::GlMesh(name) = &obj.content {
            let rot = Quat::from_euler(
                EulerRot::XYZ,
                obj.rot.x.to_radians(),
                obj.rot.y.to_radians(),
                obj.rot.z.to_radians(),
            );
            commands.spawn((
                WorldAssetRoot(
                    asset_server.load(GltfAssetLabel::Scene(0).from_asset(name.clone())),
                ), // bevy 0.19: SceneRoot → WorldAssetRoot
                Transform {
                    translation: obj.pos,
                    rotation: rot,
                    scale: obj.scale,
                },
                obj.anim(rot, false, field),
            ));
            placed.push((obj.pos, 1.0)); // rough radius (the mesh size is async); tune with MARTIN_ZOOM
            any_model = true;
            continue;
        }
        let Some(mut shaped) = shaped else { continue };
        let rot = Quat::from_euler(
            EulerRot::XYZ,
            obj.rot.x.to_radians(),
            obj.rot.y.to_radians(),
            obj.rot.z.to_radians(),
        ) * base;
        let cs = CloudSettings {
            sort_mode: SortMode::Radix,
            radix_sort_depth_bits: crate::scene::sort_depth_bits(), // MARTIN_SORT_BITS (sort cost)
            time: 0.0,
            time_start: 0.0,
            time_stop: 1.0,
            bulge: 0.0,
            global_opacity: 0.0, // animate_composition fades it in (or holds it for a entrance)
            aabb: false,         // round isotropic splats (OBB), not the conic-from-cov2d ellipse
            // MARTIN_SPLAT_SCALE shrinks every splat disk (default 1.0) → less overdraw on a weak GPU;
            // a per-object `dscale:` multiplies it (shrink just a backdrop's disks, keep the props full).
            global_scale: crate::envvar::or("MARTIN_SPLAT_SCALE", 1.0_f32)
                * obj.disk_scale.unwrap_or(1.0),
            // per-object `additive:0|1` (see build.rs) > MARTIN_ADDITIVE — for glow content on black.
            additive: obj
                .additive
                .unwrap_or_else(|| crate::envvar::or("MARTIN_ADDITIVE", 0) != 0),
            ..default()
        };
        let tf = Transform {
            translation: obj.pos,
            rotation: rot,
            scale: obj.scale,
        };
        // A `~entrance` object ASSEMBLES from a source cloud → a GaussianInterpolate (its cs.time
        // is driven by animate_composition). Without one it's a PLAIN static cloud (a fade-in) — no
        // per-frame GPU blend. Plain `PlanarGaussian3dHandle` renders directly; `calculate_bounds`
        // gives both kinds an Aabb (NO NoFrustumCulling — that would skip the Aabb → black screen).
        // `~fade`'s "source cloud" is just its shape at alpha 0 (`fade_of`), so its assemble is a pure
        // OPACITY fade — no spatial motion. Render it on the PLAIN path (no GaussianInterpolate → no
        // doubled source cloud in VRAM), only forcing the opacity fade to COMPOSE_MORPH so it still
        // fades over the same 3.6 s. Only the genuinely SPATIAL entrances (ball/scatter/swirl/…) need
        // the interpolate. (Halves a ~fade-heavy stage's resident — e.g. PonyCamp 19 ~fade objects.)
        let spatial = obj.entrance.filter(|t| *t != Entrance::Fade);
        if let Some(tr) = spatial {
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
            let mut a = obj.anim(rot, false, field);
            if obj.entrance == Some(Entrance::Fade) {
                a.fade = COMPOSE_MORPH; // preserve ~fade's 3.6 s in/out fade (was the morph duration)
            }
            commands.spawn((
                PlanarGaussian3dHandle(assets.add(PlanarGaussian3d::from(shaped))),
                Visibility::Visible,
                cs,
                tf,
                a,
            ));
        }
        placed.push((obj.pos, NORMALIZE_EXTENT * 0.5 * obj.scale.max_element()));
    }
    // Mesh props need lighting (the splats don't); a key + a soft fill so no side goes black.
    if any_model {
        commands.spawn((
            DirectionalLight {
                illuminance: 9000.0,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.9, 0.6, 0.0)),
        ));
        commands.spawn((
            DirectionalLight {
                illuminance: 3500.0,
                shadow_maps_enabled: false,
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
    crate::scene::warn_splat_budget("composition", resident);
}

/// Animate the stage from the show clock: spin + bob + drift each object, fade it in (and out, if
/// it has an `out` time) via `global_opacity`.
pub(crate) fn animate_composition(
    clock: Res<SeqClock>,
    beat: Res<crate::scene::beat::Beat>,
    // MARTIN_SIDECHAIN: the kick DUCKS the whole stage (the classic sidechain pump) — every prop's
    // brightness dips on the hit + swells back between hits. Read once; 0 = off (byte-identical).
    mut sidechain: Local<Option<f32>>,
    // CloudSettings is optional: splat objects have it (opacity fade/flare), mesh props don't (they
    // still spin/bob/drift via Transform).
    mut q: Query<(&ComposeAnim, &mut Transform, Option<&mut CloudSettings>)>,
) {
    let t = clock.t;
    let k = beat.intensity;
    let osc = (t * 0.6).sin(); // same for all objects — lift out of loop
    let beat_pulse = 1.0 + (beat.snare * 0.4 + beat.hat * 0.12) * k;
    let beat_pulse_kick = beat.kick * 0.06 * k;
    // visual sidechain: one duck factor for the whole stage (the kick pumps the camp).
    let sc = *sidechain.get_or_insert_with(|| crate::envvar::or("MARTIN_SIDECHAIN", 0.0_f32));
    let duck = if sc > 0.0 { beat.duck(sc) } else { 1.0 };
    for (a, mut tf, cs) in &mut q {
        // spin = continuous rotation; sway = a gentle oscillation around the base orientation
        // (swings a hollow-back single-image splat left/right without ever facing away).
        // Skip the trig when both are zero (static props — the common case).
        tf.rotation = a.base_rot
            * if a.spin == Vec3::ZERO && a.sway == Vec3::ZERO {
                Quat::IDENTITY
            } else {
                Quat::from_euler(
                    EulerRot::XYZ,
                    a.spin.x * t + a.sway.x * osc,
                    a.spin.y * t + a.sway.y * osc,
                    a.spin.z * t + a.sway.z * osc,
                )
            };
        let bob = if a.bob != 0.0 {
            a.bob * (t * 1.5).sin()
        } else {
            0.0
        };
        let path_off = a.path.map(|p| p.offset(t)).unwrap_or(Vec3::ZERO);
        // `travel:` eases @pos→target over its window then holds; additive with drift/path/bob so the
        // object can still bob/spin while walking in (the horse). Uses the object's `ease:` curve.
        let travel_off = a
            .travel
            .map(|tr| tr.offset(a.base_pos, a.ease, t))
            .unwrap_or(Vec3::ZERO);
        tf.translation = a.base_pos + a.drift * t + Vec3::Y * bob + path_off + travel_off;
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
        tf.scale = a.base_scale * (scale_vis * (1.0 + beat_pulse_kick));
        if let Some(mut cs) = cs {
            // a `~entrance` object assembles via the morph (cs.time) — its IN-fade is the morph, so
            // only the OUT fade touches opacity. BUT it must stay HIDDEN until its `in` cue, else its
            // source cloud (cs.time=0) renders from t=0 (a bitterbal floating through the intro).
            let started = a.appear < 0.0 || t >= a.appear;
            if a.interpolate {
                // assemble IN from the source cloud (cs.time 0→1 = cloud→object), then on `out` REVERSE
                // the morph (1→0) so the object SCATTERS back into its cloud as it fades — a stage
                // object does object→cloud→object across a scene cut, not a flat fade. Each object
                // morphs on its own (separate clouds, simultaneously) — the dreamy splat flow.
                let asm = ((t - a.appear.max(0.0)) / COMPOSE_MORPH).clamp(0.0, 1.0); // 0→1 in
                let dep = if a.out < f32::MAX {
                    ((a.out + COMPOSE_MORPH - t) / COMPOSE_MORPH).clamp(0.0, 1.0) // 1→0 on exit
                } else {
                    1.0
                };
                let op = if started { dep } else { 0.0 }; // hold opaque, fade as it scatters out
                cs.global_opacity = op * beat_pulse * duck;
                cs.time = a.ease.apply(asm.min(dep)); // formed = min(assembled, not-yet-departed)
                // per-particle reveal (pen-write tracing) only while ASSEMBLING in, not on the exit scatter.
                let (mode, soft, axis) = if asm < 1.0 {
                    a.reveal.unwrap_or((0, 0.0, 0))
                } else {
                    (0, 0.0, 0)
                };
                cs.transition_mode = mode;
                cs.transition_softness = soft;
                cs.transition_axis = axis;
            } else {
                cs.global_opacity = vis * beat_pulse * duck;
            }
            if let Some((mode, amp, freq)) = a.deform {
                cs.deform_mode = mode;
                cs.deform_amp = amp * (1.0 + (beat.kick * 0.6 + beat.snare * 0.3) * k); // per-object deform varies
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
    marks: Option<Res<crate::waypoints::Waypoints>>,
    mut cam: Query<&mut OrbitCam>,
) {
    // when a morph hero track is present it owns the camera (its own sway/flypath) — don't add a drift.
    if seq.map(|s| !s.parts.is_empty()).unwrap_or(false) {
        return;
    }
    // an authored `[camera]` track owns the camera (flypath drives it) — the auto-orbit drift would
    // fight the authored yaw, so stand down when one is present.
    if marks
        .map(|m| crate::waypoints::is_track(&m.list))
        .unwrap_or(false)
    {
        return;
    }
    if comp.map(|c| c.built).unwrap_or(false) {
        // Record: use the real per-frame step (rec.dt = 1/MARTIN_PREVIEW_FPS), NOT a hardcoded 1/60 —
        // the recorder runs one Update per frame with clock.t = i·rec.dt, so yaw += 0.12·rec.dt makes
        // the drift land at 0.12·clock.t regardless of preview fps (fps-invariant, record-safe). Live:
        // integrate real delta so the drift stays additive with the arrow keys.
        let dt = if rec.dir.is_some() {
            rec.dt
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
    fn vec3_csv_is_position_strict_and_finite() {
        assert_eq!(vec3_csv("1,2,3"), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(vec3_csv("1,2"), Vec3::new(1.0, 2.0, 0.0)); // short → trailing default
        // a bad component keeps its SLOT (no shift) — the old filter_map made `1,O,3` → (1,3,0)
        assert_eq!(vec3_csv("1,O,3"), Vec3::new(1.0, 0.0, 3.0));
        // non-finite is rejected, not passed into the Transform/camera
        let v = vec3_csv("nan,1e40,5");
        assert_eq!(v, Vec3::new(0.0, 0.0, 5.0));
        assert!(v.is_finite());
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
    fn parse_compose_forgives_a_stray_space_after_at_and_star() {
        // `@ x,y,z` / `* s` (a stray space, e.g. from column-aligned authoring) used to silently
        // collapse the object to the ORIGIN at default scale (and eat the next token). Now forgiven.
        let o = objs("mesh:bitterbal.glb @ 1.5,-0.3,0.4 * 0.5 spin 0,40,0 ~ball");
        assert_eq!(o.len(), 1);
        let s = o[0].summary();
        assert!(
            s.contains("@(1.5,-0.3,0.4)"),
            "pos not parsed past the space: {s}"
        );
        assert!(s.contains("*0.50"), "scale not parsed past the space: {s}");
        assert!(
            s.contains("~Ball"),
            "the following token must not be eaten: {s}"
        );
        // tight form still works identically.
        assert!(
            objs("mesh:bitterbal.glb @1.5,-0.3,0.4 *0.5")[0]
                .summary()
                .contains("@(1.5,-0.3,0.4)")
        );
    }

    #[test]
    fn parse_compose_reads_travel() {
        // `travel:x,y,z@anchor,dur` → an eased one-shot move; fields resolve after the token loop.
        let o = objs("mesh:bitterbal.glb @-3,0,0 *0.5 travel:1,2,3@drop,2.5");
        assert_eq!(o.len(), 1);
        let tr = o[0].travel.expect("travel parsed");
        assert_eq!(tr.target, Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(tr.dur, 2.5);
        // no @anchor + no dur → starts at the `in` cue (here none → 0) and lasts one bar.
        let q = objs("mesh:bitterbal.glb @-3,0,0 *0.5 travel:2,0,0")[0]
            .travel
            .expect("travel parsed");
        assert_eq!(q.target, Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(q.start, 0.0);
        assert!(q.dur > 0.0); // defaults to score.bar()
    }

    #[test]
    fn parse_compose_reads_field_scatter() {
        let o = objs("mesh:bitterbal.obj @0,0,0 *0.4 field:9 ~ball");
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].field, Some(9));
        assert!(o[0].summary().contains("mesh bitterbal.obj")); // field: consumed, head intact
        // a bad field value is consumed (warned), not leaked into the head.
        assert_eq!(objs("text:HI field:bad")[0].field, None);
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
