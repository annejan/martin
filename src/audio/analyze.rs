//! Spectral analysis — turn the **rendered** track into a frame-indexed table of frequency-band
//! energies, so the visuals can react to the actual *spectrum* (a pad swelling, the lead's high
//! harmonics shimmering) and not only the score's kick/snare/hat *triggers* (`scene::beat`).
//!
//! Why this is record-safe: `stream::produce` renders the whole track as a deterministic function of
//! the `Score`. We analyse that finished buffer **once** at startup into `Vec<[f32; BANDS]>` (one row
//! per 60 fps render frame), then every frame just indexes the row at `clock.t`. In record mode the
//! frame index *is* the clock, so the spectrum bakes byte-identically — same guarantee the synth and
//! the `@@anchor` times already lean on.
//!
//! Pure DSP, no deps: a hand-rolled iterative radix-2 FFT (the codebase stays dependency-light — see
//! `audio` having only fundsp), a Hann window, log-spaced bands, and per-band attack/release smoothing
//! so the bars *breathe* with the music instead of flickering like a noisy meter.

use std::f32::consts::PI;

/// Number of log-spaced frequency bands (sub → air). 8 packs into two `vec4`s for the shaders.
pub const BANDS: usize = 8;

/// FFT window length (power of two). 2048 @ 44.1 kHz ≈ 46 ms — enough low-end resolution
/// (~21.5 Hz/bin) to separate a kick's body from a bass note, short enough to stay responsive.
const WIN: usize = 2048;

/// Band edges in Hz (BANDS+1 of them), geometric from sub-bass to air. Anything below the first or
/// above the last edge is ignored (DC rumble / inaudible top).
const EDGES_HZ: [f32; BANDS + 1] = [
    30.0, 70.0, 150.0, 320.0, 700.0, 1500.0, 3500.0, 8000.0, 16000.0,
];

/// One-pole smoothing across frames. Rise fast so a hit lands on-frame; fall slow so it doesn't
/// strobe between samples. (`new > cur` → attack, else → release.)
const ATTACK: f32 = 0.6;
const RELEASE: f32 = 0.12;

/// Average interleaved-stereo `f32` (the shape `stream::produce` emits) down to mono for analysis.
pub fn mix_mono(stereo: &[f32]) -> Vec<f32> {
    stereo
        .as_chunks::<2>()
        .0
        .iter()
        .map(|s| 0.5 * (s[0] + s[1]))
        .collect()
}

/// Analyse a mono signal into per-frame band energies, one row per render frame (`frames` rows).
///
/// Each row is normalised per band to `0..1` over the whole track (so a quiet track still fills the
/// range and one band's loud transient can't flatten the others), then attack/release smoothed.
/// Fully deterministic: identical input → identical output, no clock, no RNG.
pub fn analyze(mono: &[f32], sample_rate: u32, fps: f32, frames: usize) -> Vec<[f32; BANDS]> {
    if frames == 0 {
        return Vec::new();
    }
    let sr = sample_rate as f32;
    let (hann, band_bins) = band_setup(sr);
    let fps = fps.max(1.0); // guard against zero → INF overflow

    // Pass 1: raw per-frame band energy from a window centred on each frame's sample. Each row is an
    // INDEPENDENT windowed FFT (reads only mono/hann/band_bins) → fan across rayon, the dominant cost
    // of the startup spectrum bake. `for_each_init` gives each worker its own re/im scratch (no per-row
    // alloc); writing each row by index is order-free so the result is bit-identical to the serial loop.
    let mut raw = vec![[0.0f32; BANDS]; frames];
    use rayon::prelude::*;
    raw.par_iter_mut().enumerate().for_each_init(
        || (vec![0.0f32; WIN], vec![0.0f32; WIN]),
        |(re, im), (f, row)| {
            let center = (f as f32 / fps * sr).round() as isize;
            *row = frame_bands(mono, center, &hann, &band_bins, re, im);
        },
    );

    // Pass 2: normalise each band to its own track-wide max (deterministic — the track is fixed),
    // then a mild sqrt to lift quiet detail into a visible range.
    let mut maxes = [1e-9f32; BANDS];
    for row in &raw {
        for b in 0..BANDS {
            maxes[b] = maxes[b].max(row[b]);
        }
    }
    for row in &mut raw {
        for b in 0..BANDS {
            row[b] = (row[b] / maxes[b]).clamp(0.0, 1.0).sqrt();
        }
    }

    // Pass 3: attack/release smoothing forward across frames so the bars breathe.
    let mut out = vec![[0.0f32; BANDS]; frames];
    let mut cur = [0.0f32; BANDS];
    for (f, row) in raw.iter().enumerate() {
        for b in 0..BANDS {
            let coeff = if row[b] > cur[b] { ATTACK } else { RELEASE };
            cur[b] += coeff * (row[b] - cur[b]);
        }
        out[f] = cur; // `[f32; BANDS]` is Copy — snapshot the smoothed row
    }
    out
}

/// Hann window + per-band FFT-bin ranges (built once per analysis). Shared by [`analyze`] (whole-track,
/// track-normalised) and [`StreamingAnalyzer`] (causal, AGC-normalised).
fn band_setup(sr: f32) -> (Vec<f32>, [(usize, usize); BANDS]) {
    let hann: Vec<f32> = (0..WIN)
        .map(|n| 0.5 - 0.5 * (2.0 * PI * n as f32 / WIN as f32).cos())
        .collect();
    let bin_of = |hz: f32| ((hz * WIN as f32 / sr).round() as usize).clamp(1, WIN / 2);
    let mut band_bins = [(0usize, 0usize); BANDS];
    for b in 0..BANDS {
        band_bins[b] = (
            bin_of(EDGES_HZ[b]),
            bin_of(EDGES_HZ[b + 1]).max(bin_of(EDGES_HZ[b]) + 1),
        );
    }
    (hann, band_bins)
}

/// Raw per-band RMS magnitude of one Hann-windowed FFT centred on sample `center` (zero-padded past the
/// signal ends). `re`/`im` are reusable scratch of length [`WIN`]. The per-frame core both analysers run.
fn frame_bands(
    mono: &[f32],
    center: isize,
    hann: &[f32],
    band_bins: &[(usize, usize); BANDS],
    re: &mut [f32],
    im: &mut [f32],
) -> [f32; BANDS] {
    let start = center - WIN as isize / 2;
    for n in 0..WIN {
        let idx = start + n as isize;
        let s = if idx >= 0 && (idx as usize) < mono.len() {
            mono[idx as usize]
        } else {
            0.0
        };
        re[n] = s * hann[n];
        im[n] = 0.0;
    }
    fft(re, im);
    let mut bands = [0.0f32; BANDS];
    for (b, slot) in bands.iter_mut().enumerate() {
        let (lo, hi) = band_bins[b];
        let mut power = 0.0;
        for k in lo..hi {
            power += re[k] * re[k] + im[k] * im[k];
        }
        *slot = (power / (hi - lo).max(1) as f32).sqrt();
    }
    bands
}

/// AGC peak decay per 60 fps row — a ~3 s half-life (`0.5^(1/180)`). Slow enough to hold a section's
/// level, fast enough that a quiet breakdown re-sensitises instead of being crushed by an earlier peak.
const STREAM_DECAY: f32 = 0.996_15;
/// Peak floor: keeps silence at ~0 (avoids divide-by-tiny) and lets a faint sound re-arm a decayed band.
const STREAM_FLOOR: f32 = 1e-6;

/// **Causal** spectral analyser — the streaming twin of [`analyze`]. Ingests the rendered PCM in time
/// order and emits each finalised band row the instant its FFT window is fully covered, so the spectrum
/// table fills front-to-back as the bake renders (no "whole track first" wait → no live FFT-visual dead
/// zone). Normalises per band by a running peak with slow decay (AGC) instead of the track-wide max, the
/// one change that makes it causal. A pure forward function of the PCM → deterministic / record-safe,
/// and live and record bake byte-identically since both drive it over the same `produce` output.
pub struct StreamingAnalyzer {
    sr: f32,
    fps: f32,
    hann: Vec<f32>,
    band_bins: [(usize, usize); BANDS],
    mono: Vec<f32>,
    re: Vec<f32>,
    im: Vec<f32>,
    next_frame: usize,
    peak: [f32; BANDS], // running AGC peak per band
    cur: [f32; BANDS],  // attack/release smoothing state
}

impl StreamingAnalyzer {
    pub fn new(sample_rate: u32, fps: f32) -> Self {
        let sr = sample_rate as f32;
        let (hann, band_bins) = band_setup(sr);
        Self {
            sr,
            fps: fps.max(1.0),
            hann,
            band_bins,
            mono: Vec::new(),
            re: vec![0.0; WIN],
            im: vec![0.0; WIN],
            next_frame: 0,
            peak: [STREAM_FLOOR; BANDS],
            cur: [0.0; BANDS],
        }
    }

    /// Centre sample of frame `f` (round to the nearest sample, matching `analyze`).
    fn center(&self, f: usize) -> isize {
        (f as f32 / self.fps * self.sr).round() as isize
    }

    /// One finalised, AGC-normalised + smoothed row from the current `mono` (window must be covered).
    fn emit_row(&mut self) -> [f32; BANDS] {
        let center = self.center(self.next_frame);
        let raw = frame_bands(
            &self.mono,
            center,
            &self.hann,
            &self.band_bins,
            &mut self.re,
            &mut self.im,
        );
        let mut row = [0.0f32; BANDS];
        for b in 0..BANDS {
            self.peak[b] = (self.peak[b] * STREAM_DECAY).max(raw[b]).max(STREAM_FLOOR);
            let norm = (raw[b] / self.peak[b]).clamp(0.0, 1.0).sqrt();
            let coeff = if norm > self.cur[b] { ATTACK } else { RELEASE };
            self.cur[b] += coeff * (norm - self.cur[b]);
            row[b] = self.cur[b];
        }
        self.next_frame += 1;
        row
    }

    /// Feed a chunk of interleaved-stereo PCM (the shape `produce` emits); emits every row whose FFT
    /// window is now fully inside the accumulated signal, in order.
    pub fn push_stereo(&mut self, stereo: &[f32], mut emit: impl FnMut([f32; BANDS])) {
        self.mono.extend(
            stereo
                .as_chunks::<2>()
                .0
                .iter()
                .map(|s| 0.5 * (s[0] + s[1])),
        );
        let half = WIN as isize / 2;
        while (self.center(self.next_frame) + half) as usize <= self.mono.len() {
            let row = self.emit_row();
            emit(row);
        }
    }

    /// Flush the tail: emit the remaining rows up to `total_frames` (their windows zero-pad past the
    /// signal end). Call once the render is complete.
    pub fn finish(&mut self, total_frames: usize, mut emit: impl FnMut([f32; BANDS])) {
        while self.next_frame < total_frames {
            let row = self.emit_row();
            emit(row);
        }
    }
}

/// In-place iterative radix-2 Cooley-Tukey FFT. `re`/`im` must share a power-of-two length.
/// Decimation-in-time: bit-reversal permutation, then butterflies over doubling spans.
fn fft(re: &mut [f32], im: &mut [f32]) {
    let n = re.len();
    debug_assert!(n.is_power_of_two());
    debug_assert_eq!(n, im.len());

    // Bit-reversal permutation.
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j ^= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    // Butterflies. `len` doubles each stage; twiddle = e^{-2πi/len}.
    let mut len = 2;
    while len <= n {
        let ang = -2.0 * PI / len as f32;
        let (wr_step, wi_step) = (ang.cos(), ang.sin());
        let half = len / 2;
        let mut i = 0;
        while i < n {
            let (mut wr, mut wi) = (1.0f32, 0.0f32);
            for k in 0..half {
                let a = i + k;
                let b = i + k + half;
                let tr = wr * re[b] - wi * im[b];
                let ti = wr * im[b] + wi * re[b];
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
                // advance twiddle: w *= w_step
                let nwr = wr * wr_step - wi * wi_step;
                wi = wr * wi_step + wi * wr_step;
                wr = nwr;
            }
            i += len;
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(freq: f32, secs: f32, sr: u32) -> Vec<f32> {
        let n = (secs * sr as f32) as usize;
        (0..n)
            .map(|i| (2.0 * PI * freq * i as f32 / sr as f32).sin())
            .collect()
    }

    #[test]
    fn mix_mono_averages_channels() {
        let stereo = [1.0, 0.0, 0.5, 0.5, -1.0, 1.0];
        assert_eq!(mix_mono(&stereo), vec![0.5, 0.5, 0.0]);
    }

    #[test]
    fn fft_impulse_is_flat() {
        // FFT of a unit impulse is all-ones magnitude across bins.
        let mut re = vec![0.0f32; 8];
        let mut im = vec![0.0f32; 8];
        re[0] = 1.0;
        fft(&mut re, &mut im);
        for k in 0..8 {
            let mag = (re[k] * re[k] + im[k] * im[k]).sqrt();
            assert!((mag - 1.0).abs() < 1e-5, "bin {k} mag {mag}");
        }
    }

    #[test]
    fn fft_pure_bin_concentrates_energy() {
        // A cosine at exactly bin 2 of an 8-point FFT puts all energy in bins 2 and 6 (its mirror).
        let n = 8;
        let mut re: Vec<f32> = (0..n)
            .map(|i| (2.0 * PI * 2.0 * i as f32 / n as f32).cos())
            .collect();
        let mut im = vec![0.0f32; n];
        fft(&mut re, &mut im);
        let mag = |k: usize| (re[k] * re[k] + im[k] * im[k]).sqrt();
        assert!(mag(2) > 3.0, "bin2 {}", mag(2));
        assert!(mag(6) > 3.0, "bin6 {}", mag(6));
        for k in [0usize, 1, 3, 4, 5, 7] {
            assert!(mag(k) < 1e-3, "bin {k} should be ~0, got {}", mag(k));
        }
    }

    #[test]
    fn low_tone_lights_low_band() {
        let mono = sine(60.0, 1.0, SR);
        let table = analyze(&mono, SR, 60.0, 30);
        let mid = table[15];
        assert!(mid[0] > 0.5, "sub band should be hot: {:?}", mid);
        assert!(
            mid[0] > mid[BANDS - 1] + 0.3,
            "low should dominate air: {:?}",
            mid
        );
    }

    #[test]
    fn high_tone_lights_high_band() {
        let mono = sine(9000.0, 1.0, SR);
        let table = analyze(&mono, SR, 60.0, 30);
        let mid = table[15];
        assert!(mid[BANDS - 1] > 0.5, "air band should be hot: {:?}", mid);
        assert!(
            mid[BANDS - 1] > mid[0] + 0.3,
            "air should dominate sub: {:?}",
            mid
        );
    }

    #[test]
    fn analysis_is_deterministic() {
        let mono = sine(440.0, 0.5, SR);
        let a = analyze(&mono, SR, 60.0, 20);
        let b = analyze(&mono, SR, 60.0, 20);
        assert_eq!(
            a, b,
            "same input must give bit-identical output (record-safe)"
        );
    }

    #[test]
    fn silence_is_zero_not_nan() {
        let table = analyze(&vec![0.0f32; SR as usize], SR, 60.0, 10);
        for row in table {
            for v in row {
                assert!(v.is_finite() && v.abs() < 1e-3, "silence band {v}");
            }
        }
    }

    /// Duplicate a mono buffer into interleaved stereo (L==R) — the shape the producer / analyser want.
    fn to_stereo(mono: &[f32]) -> Vec<f32> {
        mono.iter().flat_map(|&s| [s, s]).collect()
    }

    /// Run a `StreamingAnalyzer` over `stereo` split into chunks of `chunk` interleaved-stereo samples,
    /// flushing to `frames` rows. `chunk == usize::MAX` ⇒ one push.
    fn run_stream(stereo: &[f32], frames: usize, chunk: usize) -> Vec<[f32; BANDS]> {
        let mut an = StreamingAnalyzer::new(SR, 60.0);
        let mut out = Vec::new();
        for c in stereo.chunks(chunk.min(stereo.len()).max(2)) {
            an.push_stereo(c, |row| out.push(row));
        }
        an.finish(frames, |row| out.push(row));
        out
    }

    #[test]
    fn streaming_is_chunk_independent_and_deterministic() {
        // The same PCM fed in one push vs many small chunks must yield BIT-IDENTICAL rows — this is what
        // lets the live progressive feed and the record one-shot bake produce the same table (record-safe).
        // `frames` (65) exceeds the audio's ~60 frames so `finish` zero-pads the tail, exercising both paths.
        let pcm = to_stereo(&sine(440.0, 1.0, SR));
        let one = run_stream(&pcm, 65, usize::MAX);
        // even chunk (whole stereo frames, as `produce` emits), unaligned to frame/window boundaries.
        let chunked = run_stream(&pcm, 65, 910);
        assert_eq!(
            one, chunked,
            "chunked feed must equal single-push (record-safe)"
        );
        assert_eq!(
            one,
            run_stream(&pcm, 65, usize::MAX),
            "and be deterministic"
        );
        assert_eq!(one.len(), 65, "emits exactly `frames` rows");
    }

    #[test]
    fn streaming_lights_the_right_band() {
        // Sanity that the causal analyser still resolves bands (AGC-normalised): a low tone lights sub,
        // a high tone lights air — measured after the AGC peak has settled (a late row).
        let lo = run_stream(&to_stereo(&sine(60.0, 1.0, SR)), 60, usize::MAX)[40];
        assert!(
            lo[0] > lo[BANDS - 1] + 0.3,
            "low should dominate air: {lo:?}"
        );
        let hi = run_stream(&to_stereo(&sine(9000.0, 1.0, SR)), 60, usize::MAX)[40];
        assert!(
            hi[BANDS - 1] > hi[0] + 0.3,
            "air should dominate sub: {hi:?}"
        );
    }

    #[test]
    fn streaming_silence_is_finite_zero() {
        let table = run_stream(&vec![0.0f32; 2 * SR as usize], 30, usize::MAX);
        for row in table {
            for v in row {
                assert!(v.is_finite() && v.abs() < 1e-3, "silence band {v}");
            }
        }
    }

    const SR: u32 = 44_100;
}
