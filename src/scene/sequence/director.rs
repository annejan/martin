//! Per-frame logic: drive the show from `SeqClock.t` — find the active shot, retarget the single
//! interpolate entity's lhs/rhs, and set the blend factor, ball bulge, raster/entrance/deform
//! uniforms, cut-flash, beat reactions, and `glb:` dissolve.

use bevy::prelude::*;
use bevy_gaussian_splatting::morph::interpolate::GaussianInterpolate;
use bevy_gaussian_splatting::{CloudSettings, Gaussian3d, PlanarGaussian3dHandle};

use super::model::{FlashStrength, SeqState, Sequence, active_shot};
use crate::scene::effects::Entrance;
use crate::scene::gl_dissolve::gl_mesh_alpha;

const FLASH_LEN: f32 = 0.18; // cut-flash decay time (s), MARTIN_FLASH strength
const DEFORM_SPEED: f32 = 2.0; // deform animation rate: deform_time = clock.t * this
const DEPART_LEN: f32 = 1.5; // `out:` departure time (s) — carved from the end of a part's hold

/// Drive the show from `SeqClock.t`: find the active shot, retarget the interpolate entity's
/// lhs/rhs (only on change), and set the blend factor + ball bulge. Shot 0 morphs in from the
/// intro ball; every later shot morphs in from the previous shot's shape.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn shot_director(
    seq: Option<Res<Sequence>>,
    // ResMut: the director STREAMS shot clouds in/out of VRAM each shot change (ensure_window).
    state: Option<ResMut<SeqState>>,
    mut assets: ResMut<bevy::asset::Assets<bevy_gaussian_splatting::PlanarGaussian3d>>,
    clock: Res<crate::scene::SeqClock>,
    flash: Res<FlashStrength>,
    beat: Res<crate::scene::beat::Beat>,
    // the score's bar length drives `freeze:N` grid quantization (a pure fn of the clock + tempo).
    score: Res<crate::music::ScoreRes>,
    // `[sync]` look-track: when it keyframes `flash`, it overrides the global FlashStrength per frame.
    sync: Option<Res<crate::sync::SyncTrack>>,
    // (amp_scale, speed) for the persistent deform — fixed (1.0, DEFORM_SPEED); the per-shot `^name:amp`
    // token is the live strength knob, no global env var.
    mut deform_tune: Local<Option<(f32, f32)>>,
    // MARTIN_MORPH_STAGGER (0..1): per-particle staggered morph timing — the cloud DISSOLVES + reforms
    // instead of sliding (kills the straight-line streaks). Read from env once. 0 = synchronized.
    mut stagger_tune: Local<Option<f32>>,
    // MARTIN_PAIR=match → splats are pre-paired to nearest same-colour neighbours for a STRAIGHT
    // morph; the beat-driven ball-pulse (below) would fight that, so it's suppressed. Read once.
    mut pair_match: Local<Option<bool>>,
    // MARTIN_SIDECHAIN: the kick DUCKS the whole frame (the classic sidechain pump). Read once.
    mut sidechain: Local<Option<f32>>,
    mut q: Query<(
        &mut GaussianInterpolate<Gaussian3d>,
        &mut CloudSettings,
        &mut Transform,
    )>,
) {
    let (Some(seq), Some(mut state)) = (seq, state) else {
        return;
    };
    if !state.built {
        return;
    }
    let Some(entity) = state.entity else { return };
    let Ok((mut interp, mut cs, mut tf)) = q.get_mut(entity) else {
        return;
    };

    // The active shot is the last one whose absolute start time has arrived (starts come from
    // the cue timeline — `@@anchor` or laid end-to-end). It morphs in over `morph`, then holds
    // until the next shot starts. Before shot 0's start, `factor` clamps to 0 (its source state).
    let t = clock.t;
    if state.shots.is_empty() {
        return;
    }
    let idx = active_shot(state.starts(), t);
    // STREAMING: make sure {idx-1, idx, idx+1} are uploaded (and the rest dropped) before we read them.
    crate::scene::sequence::build::ensure_window(&mut state.shots, &mut assets, idx);
    let starts = state.starts();
    let shots = &state.shots;
    let s = &shots[idx];
    // Phase: ARRIVING (origin → shape), holding (shape), or DEPARTING (shape → its faded out-cloud
    // — a distinct step carved from the end of the hold, before the next shot arrives; see `out:`).
    let next = idx + 1;
    let depart_at = if next < shots.len() && s.exit.is_some() {
        (shots[next].start - DEPART_LEN).max(s.start + s.morph)
    } else {
        f32::MAX
    };
    let departing = t >= depart_at;
    // streaming uploads the active window {idx-1, idx, idx+1} above, so these are Some. Defensive
    // (compo-safe): if a cloud somehow isn't ready this frame, HOLD last frame (skip) instead of
    // panicking — a live demo must never crash on a transient.
    let (Some(cur_shape), Some(prev_shape)) = (
        s.shape.as_ref(),
        shots[idx.saturating_sub(1)].shape.as_ref(),
    ) else {
        return;
    };
    let (want_lhs, want_rhs, factor, arriving) = if departing {
        let f = ((t - depart_at) / DEPART_LEN).clamp(0.0, 1.0);
        let exit = s.exit_cloud.as_ref().unwrap_or(cur_shape);
        (cur_shape, exit, f, false)
    } else if s.entrance == Entrance::Cut {
        // hard cut: the shot is simply PRESENT from its start — factor jumps to 1.0 in one frame
        // (the first frame where t >= start, already grid-aligned since clock.t = i*dt), no blend-in.
        let lhs = match &s.origin {
            Some(h) => h,
            None => {
                debug_assert!(idx > 0, "Cut shot 0 must have an origin");
                prev_shape
            }
        };
        (lhs, cur_shape, if t >= s.start { 1.0 } else { 0.0 }, false)
    } else {
        // Shot 0 uses the full morph as offset so the logo is fully formed from frame one;
        // later shots morph from their origin (ball/fade/… → shape) like normal.
        let dt = (t - s.start) + if idx == 0 { s.morph } else { 0.0 };
        let f = (dt / s.morph.max(1e-3)).clamp(0.0, 1.0);
        // lhs: the shot's origin cloud (ball/fade/explode/…), or — for a plain Morph — the prev shape.
        let lhs = match &s.origin {
            Some(h) => h,
            None => {
                debug_assert!(idx > 0, "Morph shot 0 must have an origin");
                prev_shape
            }
        };
        (lhs, cur_shape, f, dt < s.morph)
    };
    if interp.lhs.0.id() != want_lhs.id() {
        interp.lhs = PlanarGaussian3dHandle(want_lhs.clone());
    }
    if interp.rhs.0.id() != want_rhs.id() {
        interp.rhs = PlanarGaussian3dHandle(want_rhs.clone());
    }
    let morphing = arriving || departing;
    // shape the blend factor with the shot's `ease:` curve (default Smoothstep = the old f²(3-2f)),
    // so a morph can LAND on the beat (snap/hold-snap) instead of always drifting in.
    cs.time = s.ease.apply(factor);
    // per-shot raster mode (raster:/MARTIN_RASTER): the active shot's debug shading (colour = normal).
    let want_raster = s.raster;
    if cs.rasterize_mode != want_raster {
        cs.rasterize_mode = want_raster;
    }
    // per-shot emissive glow blend (additive:/MARTIN_ADDITIVE), resolved at build time onto BuiltShot.
    if cs.additive != s.additive {
        cs.additive = s.additive;
    }
    // the ball-pulse shader effect belongs to the plain Morph entrance (prev → next through a
    // ball); source-based transitions carry their own motion, so they don't pulse.
    cs.bulge = if arriving && s.entrance == Entrance::Morph {
        s.bulge
    } else {
        0.0
    };
    // ~swarm: flock the particles along curled paths during the morph (the @_,_,N timing value is
    // the swarm strength); mutually exclusive with the ball-pulse above.
    cs.swarm = if arriving && s.entrance == Entrance::Swarm {
        s.bulge
    } else {
        0.0
    };
    // MARTIN_MORPH_STAGGER: per-particle staggered morph timing — each splat morphs over its own
    // sub-window so the cloud dissolves + reforms instead of sliding (no straight-line streaks). Safe
    // to set always: the fork shader no-ops it when the shape is held (factor 0 or 1).
    cs.morph_stagger =
        *stagger_tune.get_or_insert_with(|| crate::envvar::or("MARTIN_MORPH_STAGGER", 0.0));
    // per-particle shader transitions (typewriter/sparkle/…): drive the fork's uniforms only
    // while morphing in; otherwise mode 0 = off (held shape renders plain, fully sort-safe).
    let (mode, soft, axis) = arriving
        .then(|| s.entrance.shader_uniforms())
        .flatten()
        .unwrap_or((0, 0.0, 0));
    cs.transition_mode = mode;
    cs.transition_softness = soft;
    cs.transition_axis = axis;
    // Persistent deform (wave/cloth/ripple/twist): unlike the entrance this runs the *whole*
    // time the shot is up (not just while morphing), animated by the show clock. Mode 0 = off.
    // global deform amp scale 1.0 + speed: the per-shot `^name:amp` token is the live strength knob.
    let (amp_scale, speed) = *deform_tune.get_or_insert((1.0_f32, DEFORM_SPEED));
    let (dmode, damp, dfreq) = s.deform.map(|d| d.uniforms()).unwrap_or((0, 0.0, 0.0));
    cs.deform_mode = dmode;
    // per-shot `^name:amp` scales this shot's wobble.
    let mut amp = damp * amp_scale * s.deform_amp.unwrap_or(1.0);
    if s.deform_gated {
        // `^name@morph`: gate the amplitude to the morph — a half-sine over the transition (0 at both
        // ends, peak at the midpoint), and 0 outside it. A writhe that builds as the shape transforms
        // then goes limp once it settles (the cavia struggling as it cooks), instead of the deform
        // running the whole hold.
        amp *= if arriving {
            (std::f32::consts::PI * factor).sin()
        } else {
            0.0
        };
    }
    cs.deform_amp = amp;
    cs.deform_freq = dfreq;
    // `freeze:N` quantizes the deform playhead to N steps per bar → the wobble JUMPS on the grid
    // (stop-motion) instead of running smooth. Pure fn of clock.t + the score's bar → record-safe.
    cs.deform_time = match s.freeze {
        Some(steps) => crate::scene::effects::quantize_to_grid(t, score.0.bar(), steps) * speed,
        None => t * speed,
    };
    // Flash on each cut (term-demo's Director trick): a brief over-bright pulse at a shot's start →
    // the HDR bloom flares. Strength is the shot's own `flash:N` if set, else the global MARTIN_FLASH;
    // reuses global_opacity. When nothing flashes anywhere, skip the loop so every frame stays
    // byte-identical (the determinism guarantee).
    // global flash strength: the `[sync]` track's keyframed value if it has one, else MARTIN_FLASH.
    let global_flash = sync.as_ref().and_then(|s| s.flash_at(t)).unwrap_or(flash.0);
    let any_flash = global_flash > 0.0 || shots.iter().any(|sh| sh.flash.is_some_and(|f| f > 0.0));
    let flash = if !any_flash {
        0.0
    } else {
        shots
            .iter()
            .map(|sh| {
                let strength = sh.flash.unwrap_or(global_flash);
                let d = t - sh.start;
                if strength > 0.0 && (0.0..FLASH_LEN).contains(&d) {
                    let a = 1.0 - d / FLASH_LEN;
                    strength * a * a
                } else {
                    0.0
                }
            })
            .fold(0.0_f32, f32::max)
    };
    cs.global_opacity = 1.0 + flash;

    // Beat reactions (MARTIN_BEAT scale): the score's drum hits drive the look. A held shot can't
    // use `bulge` (it's a mid-morph ball-pulse, zero at time==1), so the kick thump rides on the
    // cloud's scale; the snare flares the bloom; kick+snare swell any active deform so a ^wave /
    // ^ripple part pumps with the track. During a morph we add a little bulge punch too.
    let pair_match = *pair_match.get_or_insert_with(|| {
        std::env::var("MARTIN_PAIR").is_ok_and(|v| v.eq_ignore_ascii_case("match"))
    });
    // per-shot `beat:<scale>` dials this shot's reaction (0 = still through the drop, >1 = punchier),
    // so the bounce can ride only on *some* shots instead of the whole show.
    let k = beat.intensity * s.beat.unwrap_or(1.0);
    if k > 0.0 {
        tf.scale = Vec3::splat(1.0 + beat.kick * 0.05 * k);
        cs.global_opacity += (beat.snare * 0.45 + beat.hat * 0.12) * k;
        // beat ball-pulse punch during a morph — but NOT under pair=match, which wants a straight slide.
        if morphing && !pair_match {
            cs.bulge += beat.kick * 0.3 * k;
        }
        if cs.deform_mode != 0 {
            cs.deform_amp *= 1.0 + (beat.kick * 0.6 + beat.snare * 0.3) * k;
        }
    }

    // Visual sidechain (MARTIN_SIDECHAIN): the kick DUCKS the whole frame — brightness dips on the
    // hit and swells back between hits, the classic sidechain "pump". Scaled by beat intensity so a
    // hushed section doesn't pump; default 0 = off (byte-identical). Deterministic (kick = clock.t).
    let sidechain =
        *sidechain.get_or_insert_with(|| crate::envvar::or("MARTIN_SIDECHAIN", 0.0_f32));
    if sidechain > 0.0 {
        cs.global_opacity *= beat.duck(sidechain);
    }

    // glb: dissolve — the splats are the exact complement of the mesh (1 − its alpha): present
    // during the splat-assemble, hidden while the mesh is crisp (no poke-through), and back as it
    // dissolves — so mesh↔splats crossfade, and the dissolve completes before the next shot morphs.
    if s.is_gl_mesh {
        cs.global_opacity *= 1.0 - gl_mesh_alpha(starts, &seq.parts, idx, t);
    }
}
