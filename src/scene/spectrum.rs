//! Spectral audio-reactivity: the **rendered track's** frequency spectrum drives the look, so a
//! backdrop breathes with the bass and shimmers on the lead's air — not just the score's kick/snare/
//! hat *triggers* ([`crate::scene::beat`]). `MARTIN_FFT=<scale>` tunes it (default `1.0`, `0` = off).
//!
//! Deterministic by construction: [`crate::audio::stream::produce`] renders the whole track as a pure
//! function of the `Score`, so we analyse it **once** into a band table at a fixed 60 rows/s
//! ([`crate::audio::analyze`]) and every frame just indexes the row at `clock.t`. Indexing by time
//! (not frame) keeps it identical across record fps (`MARTIN_PREVIEW_FPS`) and live. In **record**
//! mode the table is baked synchronously before frame 0 — a background-thread race would vary the
//! early frames and break byte-identical recordings; live mode bakes off-thread (zeros until ready).

use std::sync::{Arc, OnceLock};

use bevy::prelude::*;

use crate::audio::analyze::{self, BANDS};
use crate::audio::{self, SAMPLE_RATE};
use crate::music::ScoreRes;
use crate::scene::SeqClock;
use crate::score::Score;

/// Band table rows per second. Fixed (not the render fps) so the table is fps-independent and the
/// playhead just reads `row = clock.t * TABLE_FPS`.
const TABLE_FPS: f32 = 60.0;

/// The baked frame-indexed band table. `OnceLock` lets the live bake thread hand the table over
/// without a mutex — readers just `.get()`. `intensity == 0` ⇒ no bake, no effect.
#[derive(Resource)]
struct SpectrumTable {
    rows: Arc<OnceLock<Vec<[f32; BANDS]>>>,
    intensity: f32,
}

/// Current per-band energy at the playhead, already scaled by `MARTIN_FFT`. Read by the background +
/// interlude layers (and any future CPU consumer). The two packers feed the shared `FxUniform`.
#[derive(Resource, Default)]
pub(crate) struct Spectrum {
    pub bands: [f32; BANDS],
}

impl Spectrum {
    /// Bands 0–3 (sub … low-mid) for the first uniform `vec4`.
    pub(crate) fn as_vec4_lo(&self) -> Vec4 {
        Vec4::new(self.bands[0], self.bands[1], self.bands[2], self.bands[3])
    }
    /// Bands 4–7 (mid … air) for the second uniform `vec4`.
    pub(crate) fn as_vec4_hi(&self) -> Vec4 {
        Vec4::new(self.bands[4], self.bands[5], self.bands[6], self.bands[7])
    }
}

/// Render the whole track once and analyse it into the band table (pure fn of the score → record-safe).
fn bake(score: &Score) -> Vec<[f32; BANDS]> {
    let mut pcm: Vec<f32> = Vec::new();
    audio::stream::produce(score, |chunk| pcm.extend_from_slice(chunk));
    let mono = analyze::mix_mono(&pcm);
    let frames = (score.demo_len() * TABLE_FPS).ceil() as usize + 1;
    analyze::analyze(&mono, SAMPLE_RATE, TABLE_FPS, frames)
}

fn setup_spectrum(score: Res<ScoreRes>, mut commands: Commands) {
    let intensity = crate::envvar::or("MARTIN_FFT", 1.0_f32);
    let rows: Arc<OnceLock<Vec<[f32; BANDS]>>> = Arc::new(OnceLock::new());
    if intensity > 0.0 {
        let recording =
            std::env::var("MARTIN_RECORD").is_ok() || std::env::var("MARTIN_SHOT").is_ok();
        if recording {
            // Must be ready before the first captured frame — bake on the spot (we're offline anyway).
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| bake(&score.0)));
            if let Ok(baked) = result {
                let _ = rows.set(baked);
            } else {
                warn!("spectrum bake panicked — continuing with zero spectrum");
            }
        } else {
            // Live: bake off-thread (~7× realtime); the spectrum stays zero until it lands.
            #[cfg(not(target_arch = "wasm32"))]
            {
                let (rows, score) = (rows.clone(), score.0.clone());
                std::thread::spawn(move || {
                    let _ = rows.set(bake(&score));
                });
            }
            // wasm: no background threads — leave the table unbaked (zero spectrum, so the FFT-reactive
            // backdrops just don't pulse). Beat-driven reactivity (sidechain/cam_pump/^pulse) is separate.
        }
    }
    commands.insert_resource(SpectrumTable { rows, intensity });
}

/// Each frame: read the band row at `clock.t`, scale by `MARTIN_FFT`, into the `Spectrum` resource.
fn track_spectrum(
    table: Option<Res<SpectrumTable>>,
    clock: Res<SeqClock>,
    mut spectrum: ResMut<Spectrum>,
) {
    let Some(table) = table else { return };
    if table.intensity <= 0.0 {
        return;
    }
    let Some(rows) = table.rows.get() else { return }; // live: not baked yet → leave at zero
    if rows.is_empty() {
        return;
    }
    let idx = ((clock.t * TABLE_FPS) as usize).min(rows.len() - 1);
    for (band, &v) in spectrum.bands.iter_mut().zip(rows[idx].iter()) {
        *band = v * table.intensity;
    }
}

/// Registered by `ScenePlugin`: own the `Spectrum` resource + refresh it each frame before the layers.
pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<Spectrum>()
        .add_systems(Startup, setup_spectrum)
        .add_systems(Update, track_spectrum);
}
