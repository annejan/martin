//! Beat-gated **post-processing**: a fullscreen pass over the rendered image that RGB-splits it on
//! the kick (`MARTIN_POST=chroma`) — the whole frame shears red/cyan on every kick, the visceral
//! "the screen is locked to the track" layer.
//!
//! Deterministic + record-safe: the only input besides the current frame's pixels is `kick`, which is
//! clock-driven (`scene::beat`), so a recording bakes the exact same shear frame-for-frame.
//!
//! **TEMPORARILY STUBBED for the Bevy 0.19 migration.** 0.19's "render graph as systems" overhaul
//! removed the `ViewNode`/`ViewNodeRunner`/`RenderGraphExt`/`Node3d` API this pass was built on, so the
//! fullscreen render node is disabled pending a rewrite to the new systems-based graph. The
//! `PostSettings` component + `MARTIN_POST` parsing + the beat-drive system stay live (a `post=…` show
//! still parses and runs), so the only effect of the stub is that the chroma/grain/vignette pass draws
//! nothing — a graceful degrade on the two shows that use it (cities-defeest, campsite). See the git
//! history at this file for the 0.18 node to port. TODO(0.19): reinstate the pass via the new API.

use bevy::prelude::*;

pub(crate) const POST_CHROMA: u32 = 1;
pub(crate) const POST_GRAIN: u32 = 2;
pub(crate) const POST_VIGNETTE: u32 = 4;

/// Per-camera post-FX settings. `mode` is a BITFIELD: bit0 = chroma, bit1 = film grain, bit2 = vignette
/// (so `cine` composes all three). `kick`/`time` refresh each frame from the clock-driven beat/clock →
/// deterministic. (While the render pass is stubbed these still update; nothing reads them yet.)
#[derive(Component, Clone, Copy)]
#[allow(dead_code)] // fields feed the render pass, which is stubbed for the 0.19 migration (see above)
pub(crate) struct PostSettings {
    pub mode: u32,
    pub intensity: f32, // chroma strength (MARTIN_POST)
    pub kick: f32,      // current beat kick (0..1), set each frame
    pub time: f32,      // clock.t — deterministic film-grain animation
    pub grain: f32,     // film-grain strength
    pub vignette: f32,  // vignette strength
}

/// Parse `MARTIN_POST` into the camera's settings, or `None` if unset/off. Effects compose via `+`:
/// `chroma` · `grain` · `vignette` · `cine` (= chroma+grain+vignette preset) · `chroma+grain` etc.,
/// each optionally `:strength` (e.g. `cine:1.2`).
pub(crate) fn settings_from_env() -> Option<PostSettings> {
    let v = std::env::var("MARTIN_POST").ok()?;
    let (name, strength) = v.split_once(':').map_or((v.as_str(), 1.0), |(n, s)| {
        (n, s.trim().parse().unwrap_or(1.0))
    });
    let mut mode = 0u32;
    let (mut grain, mut vignette) = (0.0f32, 0.0f32);
    let mut chroma = 1.0f32;
    for tok in name
        .split(['+', ','])
        .map(|t| t.trim().to_ascii_lowercase())
    {
        match tok.as_str() {
            "chroma" | "rgb" | "rgb-split" | "rgbsplit" | "split" => mode |= POST_CHROMA,
            "grain" | "film" => {
                mode |= POST_GRAIN;
                grain = 0.08;
            }
            "vignette" | "vig" => {
                mode |= POST_VIGNETTE;
                vignette = 0.4;
            }
            "cine" | "cinematic" => {
                mode |= POST_CHROMA | POST_GRAIN | POST_VIGNETTE;
                chroma = 0.7;
                grain = 0.06;
                vignette = 0.32;
            }
            "off" | "" => {}
            other => {
                warn!(
                    "MARTIN_POST: unknown effect '{other}' — try chroma / grain / vignette / cine"
                )
            }
        }
    }
    if mode == 0 {
        return None;
    }
    Some(PostSettings {
        mode,
        intensity: chroma * strength,
        kick: 0.0,
        time: 0.0,
        grain: grain * strength,
        vignette: vignette * strength,
    })
}

/// Refresh each camera's `kick` from the beat (clock-driven) so the shear lands on the drum. Scaled by
/// the beat *intensity* so a hushed section tears nothing and the drop punches.
fn drive_post(
    beat: Res<crate::scene::beat::Beat>,
    clock: Res<crate::scene::SeqClock>,
    mut q: Query<&mut PostSettings>,
) {
    for mut s in &mut q {
        s.kick = beat.kick * beat.intensity;
        s.time = clock.t; // deterministic grain animation (record-safe)
    }
}

/// Beat-gated post-FX. The render pass is stubbed for the 0.19 migration (see the module doc); this
/// keeps the `PostSettings` component live + beat-driven so shows parse `post=…` and re-enabling the
/// pass later needs no show changes.
pub(crate) struct PostPlugin;

impl Plugin for PostPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, drive_post);
    }
}
