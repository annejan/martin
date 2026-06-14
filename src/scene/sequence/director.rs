//! Per-frame logic: drive the show from `SeqClock.t` — find the active part, retarget the single
//! interpolate entity's lhs/rhs, and set the blend factor, ball bulge, raster/transition/deform
//! uniforms, cut-flash, beat reactions, and `glb:` dissolve.

use bevy::prelude::*;
use bevy_gaussian_splatting::morph::interpolate::GaussianInterpolate;
use bevy_gaussian_splatting::{CloudSettings, Gaussian3d, PlanarGaussian3dHandle};

use crate::scene::content::PartContent;
use crate::scene::effects::Transition;
use crate::scene::gl_dissolve::gl_mesh_alpha;

use super::model::{FlashStrength, Sequence, SeqState, active_part};

const FLASH_LEN: f32 = 0.18; // cut-flash decay time (s), MARTIN_FLASH strength
const DEFORM_SPEED: f32 = 2.0; // deform animation rate: deform_time = clock.t * this
const DEPART_LEN: f32 = 1.5; // `out:` departure time (s) — carved from the end of a part's hold

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
    // per-part raster mode (raster:/MARTIN_RASTER): the active part's debug shading (colour = normal).
    let want_raster = state.rasters[idx];
    if cs.rasterize_mode != want_raster {
        cs.rasterize_mode = want_raster;
    }
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
    // per-particle shader transitions (typewriter/sparkle/…): drive the fork's uniforms only
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
