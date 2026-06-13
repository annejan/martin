//! Synth — the *instrument* (voices + DSP) for the placeholder track. The *score* it plays (tempo,
//! sections, drum patterns, chords, melody, dynamics) is data in `score` (`assets/score.txt`).
//! Voices are FunDSP graphs (filtered/enveloped oscillators); each is rendered + panned into a
//! **stereo** bed, sidechain-pumped under the kick, with a spread reverb send, an arp counter-line,
//! and a forward detuned lead. The whole track renders offline; martin plays it live (bevy_audio)
//! and/or writes a WAV that ffmpeg muxes onto recorded frames. (Placeholder — real track: Cinder.)
//!
//! Split by concern: `voices` (the instruments), `effects` (risers/jets/impacts/reverb/atmosphere),
//! `render` (the drums→voices→harmony→fx→master passes), and — here — the shared low-level helpers
//! (`render_into` / `vel` / `groove` / panning), `Track`, `synth_track`, and the WAV encoder.

use std::cell::Cell;
use std::sync::Arc;

use fundsp::prelude32::*;

use crate::score::{Inst, Score};

mod effects;
mod render;
mod voices;

pub const SAMPLE_RATE: u32 = 44_100;

thread_local! {
    /// `set oversample=1` → the distortion-heavy voices (saw/tanh stacks: lead, bass, supersaw, donk,
    /// house) run their oscillator+filter+shaper at 2× and downsample back, taming the aliasing those
    /// hard nonlinearities fold down at 44.1 kHz (audible as fizz in quiet/exposed parts). Off by
    /// default so the render is unchanged; set once per `synth_track` from the score.
    static OVERSAMPLE: Cell<bool> = const { Cell::new(false) };
}
pub(super) fn oversampling() -> bool {
    OVERSAMPLE.with(|c| c.get())
}

// Coarse render progress for the loader screen: one tick per completed lane/pass (the units the
// parallel render naturally falls apart into). Free-function atomics, not a resource — the render
// runs on plain threads outside the ECS.
static SYNTH_DONE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static SYNTH_TOTAL: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// `(done, total)` lane/pass units of the running synth render — `total == 0` means no render
/// started (or none pending). Read by the loader to show a real bar during the live-audio wait.
pub fn synth_progress() -> (u32, u32) {
    use std::sync::atomic::Ordering::Relaxed;
    (SYNTH_DONE.load(Relaxed), SYNTH_TOTAL.load(Relaxed))
}

fn progress_begin(total: u32) {
    use std::sync::atomic::Ordering::Relaxed;
    SYNTH_DONE.store(0, Relaxed);
    SYNTH_TOTAL.store(total, Relaxed);
}

pub(super) fn progress_tick() {
    SYNTH_DONE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// Seed a worker thread's `OVERSAMPLE` thread_local (the parallel render passes spawn threads;
/// each must carry the flag over or an `oversample=1` score would lose its anti-alias path there).
pub(super) fn set_oversampling(v: bool) {
    OVERSAMPLE.with(|c| c.set(v));
}

#[derive(Clone)]
pub struct Track {
    samples: Arc<Vec<f32>>, // interleaved stereo: L, R, L, R, …
}

impl Track {
    /// Frame count (stereo pairs) — i.e. duration·sample_rate.
    pub fn len(&self) -> usize {
        self.samples.len() / 2
    }
}

// ---- shared low-level helpers (used by `voices` / `effects` / `render`) ---------------------

/// Equal-power pan gains for `pan` in [-1, 1] (-1 = hard left, 0 = centre, 1 = hard right).
fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * (std::f32::consts::FRAC_PI_4); // 0..PI/2
    (a.cos(), a.sin())
}

/// Render a voice `node` into the interleaved-stereo `buf` at `start_t`s for `dur`s, scaled by
/// `amp` and panned by `pan`, with a 4 ms release fade so sustained voices don't click at cut-off.
pub(super) fn render_into(
    buf: &mut [f32],
    start_t: f32,
    dur: f32,
    amp: f32,
    pan: f32,
    mut node: Box<dyn AudioUnit>,
) {
    let sr = SAMPLE_RATE as f32;
    node.set_sample_rate(SAMPLE_RATE as f64);
    node.reset();
    let (lg, rg) = pan_gains(pan);
    let start = (start_t * sr) as usize;
    let n = (dur * sr) as usize;
    let rel = (0.004 * sr) as usize;
    for i in 0..n {
        let idx = start + i;
        if 2 * idx + 1 >= buf.len() {
            break;
        }
        let fade = if n > rel && i >= n - rel {
            (n - i) as f32 / rel as f32
        } else {
            1.0
        };
        let v = node.get_mono() * amp * fade;
        buf[2 * idx] += v * lg;
        buf[2 * idx + 1] += v * rg;
    }
}

pub(super) fn pseudo_noise(i: usize) -> f32 {
    // Integer hash → [-1, 1]. Robust at any sample index: an `f32 sin(i*const)` hash degrades to a
    // low-entropy near-tone for large `i` (a TL-tube buzz on late risers/impacts); this stays broadband.
    let mut n = (i as u32).wrapping_add(1).wrapping_mul(0x9E37_79B9);
    n ^= n >> 15;
    n = n.wrapping_mul(0x85EB_CA6B);
    n ^= n >> 13;
    (n as f32 / u32::MAX as f32) * 2.0 - 1.0
}

/// Per-note VELOCITY from the metric 16th-slot position + a deterministic hash: downbeats accent,
/// the back-beat next, off-beats soften, weak 16ths ghost — with ±15% humanizing jitter. Multiplied
/// into every voice's render amp (and the filter brightness) so the track breathes like a performance
/// instead of the flat, every-note-identical GM wall that reads as cheap.
pub(super) fn vel(t: f32, beat: f32, seed: u32) -> f32 {
    let sl = beat / 4.0;
    let slot = ((t / sl).round() as i64).rem_euclid(16) as usize;
    let metric = match slot {
        0 => 1.0,
        8 => 0.94,
        4 | 12 => 0.84,
        2 | 6 | 10 | 14 => 0.68,
        _ => 0.52,
    };
    let h = pseudo_noise((t * 9973.0) as usize ^ seed as usize) * 0.5 + 0.5; // 0..1
    (metric * (0.85 + 0.30 * h)).clamp(0.25, 1.0)
}

/// Humanize an onset time: swing the odd 16ths late + lay the lane back a touch + a little jitter, so
/// the groove pushes/pulls instead of sitting dead on the quantize grid (the second machine tell). The
/// kick and the sidechain source stay dead-on — only the bed voices are grooved.
pub(super) fn groove(t: f32, beat: f32, seed: u32, jit: f32, lay: f32) -> f32 {
    let sl = beat / 4.0;
    let s = (t / sl).round() as i64;
    let swing = if s.rem_euclid(2) == 1 { 0.10 * sl } else { 0.0 };
    let j = pseudo_noise((t * 4099.0) as usize ^ seed as usize) * jit;
    (t + swing + lay + j).max(0.0)
}

pub(super) fn add_stereo(buf: &mut [f32], frame: usize, v: f32, pan: f32) {
    if 2 * frame + 1 >= buf.len() {
        return;
    }
    let (lg, rg) = pan_gains(pan);
    buf[2 * frame] += v * lg;
    buf[2 * frame + 1] += v * rg;
}

/// Render the triad as three voices panned across the field (wide chords), via `voice(freq)`.
pub(super) fn chord_spread(
    buf: &mut [f32],
    t: f32,
    dur: f32,
    amp: f32,
    spread: f32,
    tri: [f32; 3],
    voice: fn(f32) -> Box<dyn AudioUnit>,
) {
    for (i, &f) in tri.iter().enumerate() {
        let pan = (i as f32 - 1.0) * spread; // -spread, 0, +spread
        render_into(buf, t, dur, amp, pan, voice(f));
    }
}

/// Keep a chord root in the deep sub range. Score roots are parsed around octave 3; the sub layer
/// wants the same pitch class down around 24-38 Hz, with an added harmonic later for translation on
/// smaller speakers.
pub(super) fn sub_freq(root: f32) -> f32 {
    let mut f = root;
    while f > 38.0 {
        f *= 0.5;
    }
    while f < 24.0 {
        f *= 2.0;
    }
    f
}

/// Punchier bass voice above the sub, locked to the same chord-root pitch class.
pub(super) fn bass_freq(root: f32) -> f32 {
    sub_freq(root) * 2.0
}

pub(super) fn section_time(score: &Score, name: &str) -> Option<f32> {
    score
        .sections
        .iter()
        .position(|s| s.name == name)
        .map(|i| score.section_start_secs(i))
}

/// `(start, end)` seconds of a named section (end = next section's start, or the demo end).
pub(super) fn section_window(score: &Score, name: &str) -> Option<(f32, f32)> {
    let i = score.sections.iter().position(|s| s.name == name)?;
    let start = score.section_start_secs(i);
    let end = if i + 1 < score.sections.len() {
        score.section_start_secs(i + 1)
    } else {
        score.demo_len()
    };
    Some((start, end))
}

/// Render the whole score to an interleaved-stereo buffer: voices panned into a "bed" (everything
/// but the kick), an arp counter-line in the energetic sections, sidechain pump under the kick, a
/// spread reverb send, the continuous sub, per-section fades × gain, soft clip.
///
/// This is a WHOLE-TRACK, in-memory render (a handful of `demo_len`-sized buffers, ~tens of MB
/// each), not a streaming/block one — the spread reverb runs global feedback combs over the entire
/// bed and the master wants the whole signal. But it is a **multi-core** render: the four content
/// passes (drums / voices / harmony / fx) are independent write-only accumulators, so they render
/// concurrently into their own buffers and are summed afterwards **in the original pass order**,
/// which keeps the floating-point result bit-for-bit identical to the old sequential accumulation
/// (`(((drums+voices)+harmony)+fx)` per sample — exactly what the ordered `bed[i] += …` produced).
/// Only the master pass (sidechain/reverb/limiter, cheap O(n) filters) stays serial. Live playback
/// holds the show clock until this finishes (`music.rs` AudioGate), so picture + music start
/// sample-locked at t=0; `MARTIN_MUSIC` (the bundle's pre-rendered WAV) skips the wait entirely.
pub fn synth_track(score: &Score) -> Track {
    let oversample = score.param("oversample", 0.0) > 0.5; // `set oversample=1` — anti-alias
    OVERSAMPLE.with(|c| c.set(oversample));
    progress_begin(11); // drums + 4 voice lanes + 4 harmony lanes + fx + master
    let sr = SAMPLE_RATE as f32;
    let total = (score.demo_len() * sr).ceil() as usize;
    let stereo = total * 2;
    let mut kickbuf = vec![0f32; stereo]; // sidechain source (never ducked)
    let mut bed = vec![0f32; stereo]; // drums land here — the first accumulator (0 + drums ≡ drums)
    let mut bed_voices = vec![0f32; stereo];
    let mut bed_harmony = vec![0f32; stereo];
    let mut bed_fx = vec![0f32; stereo];

    let kicks = score.hits(Inst::Kick);
    // OVERSAMPLE is a thread_local — each worker must set it itself, or an `oversample=1` score
    // would silently lose its anti-alias path on the threaded passes.
    std::thread::scope(|s| {
        s.spawn(|| {
            OVERSAMPLE.with(|c| c.set(oversample));
            render::render_voices(&mut bed_voices, score, stereo);
        });
        s.spawn(|| {
            OVERSAMPLE.with(|c| c.set(oversample));
            render::render_harmony(&mut bed_harmony, score);
        });
        s.spawn(|| {
            OVERSAMPLE.with(|c| c.set(oversample));
            render::render_fx(&mut bed_fx, score, total);
            progress_tick();
        });
        render::render_drums(&mut kickbuf, &mut bed, score, &kicks);
        progress_tick();
    });
    for i in 0..stereo {
        bed[i] = ((bed[i] + bed_voices[i]) + bed_harmony[i]) + bed_fx[i];
    }
    let buf = render::master(&kickbuf, &mut bed, score, &kicks, total, stereo);
    progress_tick();
    Track {
        samples: Arc::new(buf),
    }
}

/// Encode the track as a 16-bit PCM **stereo** WAV (`SAMPLE_RATE`) into a byte buffer — hand-rolled
/// RIFF header, no audio dependency. Reused for the on-disk WAV (`write_wav`) and live playback.
pub fn encode_wav(track: &Track) -> Vec<u8> {
    let data_bytes = (track.samples.len() * 2) as u32; // interleaved samples × 2 bytes
    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // format = PCM
    out.extend_from_slice(&2u16.to_le_bytes()); // channels = stereo
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes()); // sample rate
    out.extend_from_slice(&(SAMPLE_RATE * 4).to_le_bytes()); // byte rate (rate × block align)
    out.extend_from_slice(&4u16.to_le_bytes()); // block align (2 ch × 2 bytes)
    out.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for &s in track.samples.iter() {
        out.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

/// Write the track as a `.wav` file so ffmpeg can mux it onto the recorded frames.
pub fn write_wav(track: &Track, path: &str) -> std::io::Result<()> {
    std::fs::write(path, encode_wav(track))
}

#[cfg(test)]
mod voice_demo {
    use std::sync::Arc;

    use super::voices::woozbass;
    use super::*;

    /// On-demand audition of `woozbass`: writes a few HELD notes to /tmp/woozbass.wav so the slow
    /// growl + wooze can be heard. Run with:
    ///   cargo +nightly test --release woozbass_demo -- --ignored
    #[test]
    #[ignore]
    fn woozbass_demo() {
        let sr = SAMPLE_RATE as f32;
        let mut bed = vec![0f32; (7.0 * sr) as usize * 2];
        // low fundamentals (A1..E2) held ~1 s each — long enough for the growl to develop.
        let notes = [55.0f32, 73.42, 49.0, 82.41, 65.41, 55.0];
        for (i, &f) in notes.iter().enumerate() {
            render_into(&mut bed, i as f32 * 1.1, 1.0, 0.85, 0.0, woozbass(f));
        }
        let track = Track {
            samples: Arc::new(bed),
        };
        write_wav(&track, "/tmp/woozbass.wav").expect("write demo wav");
    }
}
