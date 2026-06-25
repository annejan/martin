//! Spectral audio-reactivity: the **rendered track's** frequency spectrum drives the look, so a
//! backdrop breathes with the bass and shimmers on the lead's air — not just the score's kick/snare/
//! hat *triggers* ([`crate::scene::beat`]). `MARTIN_FFT=<scale>` tunes it (default `1.0`, `0` = off).
//!
//! Deterministic by construction: the synth renders the track as a pure function of the `Score`, so the
//! band table (60 rows/s) is a pure function too and every frame just indexes the row at `clock.t`
//! (indexing by time, not frame, keeps it identical across record fps and live).
//!
//! Live uses a **causal streaming** analyser ([`crate::audio::analyze::StreamingAnalyzer`], AGC-
//! normalised): a background thread renders the score in time order and APPENDS each band row the moment
//! its FFT window is covered, so the table fills front-to-back ahead of the playhead — the FFT-reactive
//! visuals come alive from t=0 (no dead zone). **Record** mode bakes the full table synchronously before
//! frame 0 (a background-thread race would vary the early frames and break byte-identical recordings).
//! `MARTIN_FFT_NORM=track` selects the legacy whole-track normalisation ([`crate::audio::analyze`]) — it
//! needs the whole track first, so it re-introduces the live dead zone; kept for exact old output.

use std::sync::{Arc, Mutex};

use bevy::prelude::*;

use crate::audio::analyze::{self, BANDS, StreamingAnalyzer};
use crate::audio::{self, SAMPLE_RATE};
use crate::music::ScoreRes;
use crate::scene::SeqClock;
use crate::score::Score;

/// Band table rows per second. Fixed (not the render fps) so the table is fps-independent and the
/// playhead just reads `row = clock.t * TABLE_FPS`.
const TABLE_FPS: f32 = 60.0;

/// The baked frame-indexed band table. A `Mutex<Vec>` rather than a set-once `OnceLock` so the live
/// **streaming** bake can fill it PROGRESSIVELY (front-to-back) — readers index up to its current
/// length, so the FFT-reactive visuals come alive from t=0 instead of waiting for the whole-track bake
/// (the old dead zone). `intensity == 0` ⇒ no bake, no effect.
#[derive(Resource)]
struct SpectrumTable {
    rows: Arc<Mutex<Vec<[f32; BANDS]>>>,
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

fn total_frames(score: &Score) -> usize {
    (score.demo_len() * TABLE_FPS).ceil() as usize + 1
}

/// True when `MARTIN_FFT_NORM=track` selects the legacy whole-track normalisation (each band scaled to
/// its track-wide max). It needs the whole track first, so it re-introduces the live FFT-visual dead
/// zone — kept only for byte-identical-to-old output. Default = the causal AGC streaming analyser.
fn track_norm() -> bool {
    std::env::var("MARTIN_FFT_NORM").is_ok_and(|v| v.eq_ignore_ascii_case("track"))
}

/// RECORD path (offline, synchronous before frame 0): render the whole track FAST (rayon
/// `produce_parallel`) then analyse it in ONE shot into the full table. Streaming-AGC by default
/// (matches the live look), or the legacy track-normalised `analyze` under `MARTIN_FFT_NORM=track`. No
/// dead-zone concern here — it's done before the first captured frame, so we optimise for speed.
fn bake_full(score: &Score) -> Vec<[f32; BANDS]> {
    let mut pcm: Vec<f32> = Vec::new();
    audio::stream::produce_parallel(score, |chunk| pcm.extend_from_slice(chunk));
    let frames = total_frames(score);
    if track_norm() {
        let mono = analyze::mix_mono(&pcm);
        return analyze::analyze(&mono, SAMPLE_RATE, TABLE_FPS, frames);
    }
    // Streaming analyser over the whole buffer in one push — chunk-boundary-independent, so this is
    // byte-identical to the live progressive feed (modulo the produce vs produce_parallel epsilon).
    let mut an = StreamingAnalyzer::new(SAMPLE_RATE, TABLE_FPS);
    let mut out = Vec::with_capacity(frames);
    an.push_stereo(&pcm, |row| out.push(row));
    an.finish(frames, |row| out.push(row));
    out
}

fn setup_spectrum(score: Res<ScoreRes>, mut commands: Commands) {
    let intensity = crate::envvar::or("MARTIN_FFT", 1.0_f32);
    let rows: Arc<Mutex<Vec<[f32; BANDS]>>> = Arc::new(Mutex::new(Vec::new()));
    if intensity > 0.0 {
        let recording =
            std::env::var("MARTIN_RECORD").is_ok() || std::env::var("MARTIN_SHOT").is_ok();
        if recording {
            // Must be ready before the first captured frame — bake on the spot (we're offline anyway).
            let result =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| bake_full(&score.0)));
            match result {
                Ok(baked) => *rows.lock().unwrap() = baked,
                Err(_) => warn!("spectrum bake panicked — continuing with zero spectrum"),
            }
        } else if track_norm() {
            // Legacy whole-track normalisation: bake off-thread, lands all at once (the old dead zone).
            let (rows, score) = (rows.clone(), score.0.clone());
            std::thread::spawn(move || {
                let baked = bake_full(&score);
                *rows.lock().unwrap() = baked;
            });
        } else {
            // LIVE default: stream the score in time order on a background thread, APPENDING each
            // finalised row as its FFT window is covered. Single-threaded `produce` emits early segments
            // first and faster-than-realtime, so the table fills well ahead of the playhead → the
            // FFT-reactive visuals react from t=0 (no dead zone). The AGC normalisation makes it causal.
            let (rows, score) = (rows.clone(), score.0.clone());
            std::thread::spawn(move || {
                let frames = total_frames(&score);
                let mut an = StreamingAnalyzer::new(SAMPLE_RATE, TABLE_FPS);
                audio::stream::produce(&score, |chunk| {
                    let mut batch = Vec::new();
                    an.push_stereo(chunk, |row| batch.push(row));
                    if !batch.is_empty() {
                        rows.lock().unwrap().extend(batch);
                    }
                });
                let mut tail = Vec::new();
                an.finish(frames, |row| tail.push(row));
                rows.lock().unwrap().extend(tail);
            });
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
    let rows = table.rows.lock().unwrap();
    if rows.is_empty() {
        return; // not baked yet (or first streaming rows not in) → leave at zero
    }
    // Index up to the CURRENTLY-FILLED length: the live streaming bake appends front-to-back, so a row
    // for the playhead is normally already in (the bake runs ahead of realtime); clamp covers the rare
    // case the playhead briefly outruns the fill.
    let idx = ((clock.t * TABLE_FPS) as usize).min(rows.len() - 1);
    let row = rows[idx];
    drop(rows);
    for (band, &v) in spectrum.bands.iter_mut().zip(row.iter()) {
        *band = v * table.intensity;
    }
}

/// Registered by `ScenePlugin`: own the `Spectrum` resource + refresh it each frame before the layers.
pub(crate) fn plugin(app: &mut App) {
    app.init_resource::<Spectrum>()
        .add_systems(Startup, setup_spectrum)
        .add_systems(Update, track_spectrum);
}
