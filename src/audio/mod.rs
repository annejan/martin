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

use crate::score::Score;

pub(crate) mod analyze;
mod effects;
mod render;
pub(crate) mod stream;
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

// Streaming-render progress for the loader screen: how many stereo frames the live `stream::produce`
// has finalized. Free-function atomics, not a resource — the producer runs on a plain thread.
static SYNTH_DONE_FRAMES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Stereo frames the running stream producer has finalized so far (0 before it starts).
pub fn synth_produced_frames() -> usize {
    SYNTH_DONE_FRAMES.load(std::sync::atomic::Ordering::Acquire)
}

/// Playback buffer to accumulate before the show starts (≈2 s); at ~7× realtime the producer fills
/// it in a fraction of a second and then stays well ahead — the loader covers this brief wait.
pub const STREAM_LEAD_FRAMES: usize = 2 * SAMPLE_RATE as usize;

pub(super) fn progress_reset() {
    SYNTH_DONE_FRAMES.store(0, std::sync::atomic::Ordering::Release);
}
pub(super) fn progress_advance(frames: usize) {
    SYNTH_DONE_FRAMES.fetch_add(frames, std::sync::atomic::Ordering::Release);
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
pub(crate) fn vel(t: f32, beat: f32, seed: u32) -> f32 {
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
/// wants the same pitch class folded into a 19-38 Hz window. The bounds span exactly one octave
/// (38/19 = 2), so EVERY pitch class lands inside — a sub-octave window let D#/E/F escape above the
/// top (E rang at ~41 Hz instead of a true sub). An added harmonic later aids small-speaker translation.
pub(super) fn sub_freq(root: f32) -> f32 {
    let mut f = if root.is_finite() {
        root.abs().max(1e-6)
    } else {
        27.5
    };
    while f > 38.0 {
        f *= 0.5;
    }
    while f < 19.0 {
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

/// Render the whole score to an interleaved-stereo buffer (the deliverable: recordings via
/// `MARTIN_SYNTH_WAV` + the bundle's pre-rendered WAV).
///
/// There is **one** DSP engine: `stream::produce`. This batch entry just runs it to completion with
/// a collecting sink, so the recorded track is sample-for-sample what live playback streams — no
/// second implementation to keep in sync. (Historically there were two — a multi-core whole-track
/// render here and the segmented streamer — kept matching "by ear" + a tolerance test; they agreed
/// to within a single 16-bit LSB, so the streamer is now the sole source of truth.)
pub fn synth_track(score: &Score) -> Track {
    let mut samples = Vec::new();
    // batch render → spread the 10 lanes' note voices across cores (live playback uses `produce`).
    stream::produce_parallel(score, |chunk| samples.extend_from_slice(chunk));
    Track {
        samples: Arc::new(samples),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pan_gains_are_equal_power() {
        // centre = -3 dB on both legs (cos = sin = 1/√2), so lg² + rg² == 1 at every pan.
        for &pan in &[-1.0, -0.5, 0.0, 0.37, 1.0, 2.0, -9.0] {
            let (l, r) = pan_gains(pan);
            assert!((l * l + r * r - 1.0).abs() < 1e-5, "pan {pan}: {l},{r}");
        }
        let (l, r) = pan_gains(-1.0);
        assert!(l > 0.999 && r < 1e-5); // hard left
        let (l, r) = pan_gains(1.0);
        assert!(l < 1e-5 && r > 0.999); // hard right
        let (l, r) = pan_gains(0.0);
        assert!((l - r).abs() < 1e-6); // centred
    }

    #[test]
    fn vel_clamps_accents_downbeats_and_is_deterministic() {
        let beat = 1.0; // 16th slot = 0.25 s
        // range invariant across a bar of 16ths
        for k in 0..16 {
            let v = vel(k as f32 * 0.25, beat, 0x55);
            assert!((0.25..=1.0).contains(&v), "slot {k}: {v}");
        }
        // metric weighting dominates the ±jitter: a downbeat always beats a weak 16th, whatever the
        // hash rolls (min downbeat 0.85 > max weak-16th 0.598).
        assert!(vel(0.0, beat, 0x55) > vel(0.25, beat, 0x55));
        // pure fn of (t, beat, seed) — the determinism guarantee the recorder relies on.
        assert_eq!(vel(0.5, beat, 0x77), vel(0.5, beat, 0x77));
    }

    #[test]
    fn sub_and_bass_freq_fold_to_their_windows() {
        // The sub window [19,38] spans exactly one octave (38/19 = 2), so the fold is CLOSED — every
        // pitch class lands inside, including D#/E/F (E1 = 41.2 → 20.6), which a sub-octave window let
        // escape above the top. Bass sits exactly an octave above. (41.2 = E1, 82.4 = E2.)
        for &root in &[27.5f32, 41.2, 55.0, 82.4, 110.0, 220.0, 440.0, 1000.0, 30.0] {
            let s = sub_freq(root);
            assert!((19.0..=38.0).contains(&s), "root {root} → sub {s}");
            assert_eq!(bass_freq(root), s * 2.0);
            let ratio = (root / s).log2(); // same pitch class: ratio is a power of two
            assert!((ratio - ratio.round()).abs() < 1e-4, "root {root} sub {s}");
        }
    }

    #[test]
    fn pseudo_noise_stays_in_unit_range() {
        for i in [0usize, 1, 7, 1000, 44_100, usize::MAX / 2] {
            let n = pseudo_noise(i);
            assert!((-1.0..=1.0).contains(&n), "i {i}: {n}");
            assert_eq!(n, pseudo_noise(i)); // deterministic
        }
    }

    #[test]
    fn encode_wav_writes_a_valid_riff_header() {
        // one stereo frame (L=full-scale, R=clamped-from-overrange)
        let track = Track {
            samples: Arc::new(vec![1.0, 2.0]),
        };
        let w = encode_wav(&track);
        assert_eq!(w.len(), 44 + 4); // header + 2 samples × 2 bytes
        assert_eq!(&w[0..4], b"RIFF");
        assert_eq!(&w[8..12], b"WAVE");
        assert_eq!(&w[12..16], b"fmt ");
        assert_eq!(&w[36..40], b"data");
        assert_eq!(u16::from_le_bytes([w[22], w[23]]), 2); // stereo
        assert_eq!(
            u32::from_le_bytes([w[24], w[25], w[26], w[27]]),
            SAMPLE_RATE
        );
        assert_eq!(u32::from_le_bytes([w[40], w[41], w[42], w[43]]), 4); // data bytes
        // both samples clamp to +full-scale (1.0 and the over-range 2.0)
        assert_eq!(i16::from_le_bytes([w[44], w[45]]), 32767);
        assert_eq!(i16::from_le_bytes([w[46], w[47]]), 32767);
    }

    // ---- synth-DSP end-to-end sanity (voices → effects → render → master via produce_parallel) -----
    // A small score keeps these fast; they guard the whole synth path against silent corruption — the
    // failure mode unit tests on the helpers alone can't catch (a NaN/blow-up/silence/non-determinism
    // only emerging once the full lane mix runs).

    /// A short, valid score (~8 s of audio) so the full-synth tests render quickly.
    fn short_score() -> Score {
        Score::from_str("bpm 120\nchords C Am F G\nsection drop 4 4\n").expect("valid score")
    }

    #[test]
    fn synth_track_is_finite_bounded_and_non_silent() {
        let track = synth_track(&short_score());
        assert!(track.len() > 0, "empty track");
        let mut peak = 0f32;
        for &s in track.samples.iter() {
            assert!(s.is_finite(), "non-finite sample in the synth output: {s}");
            peak = peak.max(s.abs());
        }
        // The WAV encoder clamps to ±1, but the float bus may ride a little hot through the master
        // chain — a sane mix still stays well under a few full-scales (catches a runaway feedback/gain).
        assert!(peak < 4.0, "peak {peak} — the master bus blew up");
        // …and there IS signal (not silence / all-DC) — the chords drive the pad/bass voices.
        assert!(peak > 0.02, "peak {peak} — the track is ~silent");
    }

    #[test]
    fn synth_track_duration_matches_the_score() {
        let score = short_score();
        let secs = synth_track(&score).len() as f32 / SAMPLE_RATE as f32;
        let want = score.demo_len();
        assert!(
            (secs - want).abs() < 2.0,
            "rendered {secs:.2}s vs score demo_len {want:.2}s"
        );
    }

    #[test]
    fn synth_track_is_deterministic_within_epsilon() {
        // produce_parallel sums the lanes across threads, so it isn't bit-identical run-to-run, but it
        // must stay within a tiny epsilon — recordings + live playback must not audibly drift.
        let score = short_score();
        let a = synth_track(&score);
        let b = synth_track(&score);
        assert_eq!(a.len(), b.len(), "length drift between runs");
        let max_diff = a
            .samples
            .iter()
            .zip(b.samples.iter())
            .map(|(x, y)| (x - y).abs())
            .fold(0f32, f32::max);
        assert!(
            max_diff < 1e-3,
            "max sample diff {max_diff} — synth not deterministic enough"
        );
    }
}
