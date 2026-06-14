//! Per-frame logic: drive the show from `SeqClock.t` — find the active shot, retarget the single
//! interpolate entity's lhs/rhs, and set the blend factor, ball bulge, raster/transition/deform
//! uniforms, cut-flash, beat reactions, and `glb:` dissolve.

use bevy::prelude::*;
use bevy_gaussian_splatting::morph::interpolate::GaussianInterpolate;
use bevy_gaussian_splatting::{CloudSettings, Gaussian3d, PlanarGaussian3dHandle};

use super::model::{FlashStrength, SeqState, Sequence, active_shot};
use crate::scene::effects::Transition;
use crate::scene::gl_dissolve::gl_mesh_alpha;

const FLASH_LEN: f32 = 0.18; // cut-flash decay time (s), MARTIN_FLASH strength
const DEFORM_SPEED: f32 = 2.0; // deform animation rate: deform_time = clock.t * this
const DEPART_LEN: f32 = 1.5; // `out:` departure time (s) — carved from the end of a part's hold

/// Drive the show from `SeqClock.t`: find the active shot, retarget the interpolate entity's
/// lhs/rhs (only on change), and set the blend factor + ball bulge. Shot 0 morphs in from the
/// intro ball; every later shot morphs in from the previous shot's shape.
#[allow(clippy::type_complexity)]
pub(crate) fn shot_director(
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
    let shots = &state.shots;
    let starts = state.starts();

    // The active shot is the last one whose absolute start time has arrived (starts come from
    // the cue timeline — `@@anchor` or laid end-to-end). It morphs in over `morph`, then holds
    // until the next shot starts. Before shot 0's start, `factor` clamps to 0 (its source state).
    let t = clock.t;
    let idx = active_shot(&starts, t);
    let s = &shots[idx];
    // Phase: ARRIVING (origin → shape), holding (shape), or DEPARTING (shape → its faded out-cloud
    // — a distinct step carved from the end of the hold, before the next shot arrives; see `out:`).
    let next = idx + 1;
    let depart_at = if next < shots.len() && s.out.is_some() {
        (shots[next].start - DEPART_LEN).max(s.start + s.morph)
    } else {
        f32::MAX
    };
    let departing = t >= depart_at;
    let (want_lhs, want_rhs, factor, arriving) = if departing {
        let f = ((t - depart_at) / DEPART_LEN).clamp(0.0, 1.0);
        let out = s.out_cloud.as_ref().unwrap_or(&s.shape);
        (&s.shape, out, f, false)
    } else {
        let dt = t - s.start;
        let f = (dt / s.morph.max(1e-3)).clamp(0.0, 1.0);
        // lhs: the shot's origin cloud (ball/fade/explode/…), or — for a plain Morph — the prev shape.
        let lhs = match &s.origin {
            Some(h) => h,
            None => &shots[idx - 1].shape,
        };
        (lhs, &s.shape, f, dt < s.morph)
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
    // per-shot raster mode (raster:/MARTIN_RASTER): the active shot's debug shading (colour = normal).
    let want_raster = s.raster;
    if cs.rasterize_mode != want_raster {
        cs.rasterize_mode = want_raster;
    }
    // the ball-pulse shader effect belongs to the plain Morph transition (prev → next through a
    // ball); source-based transitions carry their own motion, so they don't pulse.
    cs.bulge = if arriving && s.transition == Transition::Morph {
        s.bulge
    } else {
        0.0
    };
    // ~swarm: flock the particles along curled paths during the morph (the @_,_,N timing value is
    // the swarm strength); mutually exclusive with the ball-pulse above.
    cs.swarm = if arriving && s.transition == Transition::Swarm {
        s.bulge
    } else {
        0.0
    };
    // per-particle shader transitions (typewriter/sparkle/…): drive the fork's uniforms only
    // while morphing in; otherwise mode 0 = off (held shape renders plain, fully sort-safe).
    let (mode, soft, axis) = arriving
        .then(|| s.transition.shader_uniforms())
        .flatten()
        .unwrap_or((0, 0.0, 0));
    cs.transition_mode = mode;
    cs.transition_softness = soft;
    cs.transition_axis = axis;
    // Persistent deform (wave/cloth/ripple/twist): unlike the transition this runs the *whole*
    // time the shot is up (not just while morphing), animated by the show clock. Mode 0 = off.
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
    let (dmode, damp, dfreq) = s.deform.map(|d| d.uniforms()).unwrap_or((0, 0.0, 0.0));
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
                .map(|&start| {
                    let d = t - start;
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

    // Beat reactions (MARTIN_BEAT scale): the score's drum hits drive the look. A held shot can't
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
    // dissolves — so mesh↔splats crossfade, and the dissolve completes before the next shot morphs.
    if s.is_gl_mesh {
        cs.global_opacity *= 1.0 - gl_mesh_alpha(&starts, &seq.parts, idx, t);
    }
}
