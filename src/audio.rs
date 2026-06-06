//! Synth — the *instrument* (voices + DSP) for the placeholder track. The *score* it plays (tempo,
//! sections, drum patterns, chords, melody, dynamics) is data in `score.rs` (`assets/score.txt`).
//! Voices are FunDSP graphs (filtered/enveloped oscillators); each is rendered + panned into a
//! **stereo** bed, sidechain-pumped under the kick, with a spread reverb send, an arp counter-line,
//! and a forward detuned lead. The whole track renders offline; martin plays it live (bevy_audio)
//! and/or writes a WAV that ffmpeg muxes onto recorded frames. (Placeholder — real track: Cinder.)

use std::sync::Arc;

use fundsp::prelude32::*;

use crate::score::{Inst, Score};

pub const SAMPLE_RATE: u32 = 44_100;

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

// ---- voices (FunDSP graphs; each is a 0-input → 1-output unit) ------------------------------

/// Kick: a sine swept from ~125 Hz down to 45 Hz with a fast amplitude decay.
fn kick() -> Box<dyn AudioUnit> {
    Box::new(
        (envelope(|t: f32| 45.0 + 80.0 * (-t * 38.0).exp()) >> sine())
            * envelope(|t: f32| (-t * 9.0).exp()),
    )
}

/// Snare: high-passed noise burst + a short tone body.
fn snare() -> Box<dyn AudioUnit> {
    Box::new(
        ((noise() >> highpass_hz(1200.0, 0.7)) * envelope(|t: f32| (-t * 26.0).exp())
            + sine_hz(190.0) * envelope(|t: f32| (-t * 24.0).exp()) * 0.5)
            * 0.7,
    )
}

/// Hat: very short bright high-passed noise.
fn hat() -> Box<dyn AudioUnit> {
    Box::new((noise() >> highpass_hz(7000.0, 0.7)) * envelope(|t: f32| (-t * 80.0).exp()))
}

/// Stab: one chord note as a saw through a low-pass with a plucky attack (rendered per triad note
/// so the three can be panned wide).
fn stab(freq: f32) -> Box<dyn AudioUnit> {
    Box::new(
        (saw_hz(freq) >> lowpass_hz(1600.0, 0.8))
            * envelope(|t: f32| {
                let a = 0.01;
                if t < a {
                    t / a
                } else {
                    (-(t - a) * 7.0).exp()
                }
            })
            * 0.3,
    )
}

/// Pad: one chord note an octave down through a soft low-pass, slow swell (panned per note for width).
fn pad(freq: f32) -> Box<dyn AudioUnit> {
    Box::new(
        (saw_hz(freq * 0.5) >> lowpass_hz(900.0, 0.6))
            * envelope(|t: f32| (t * 2.0).min(1.0))
            * 0.22,
    )
}

/// Bass: sine + a touch of saw through a low-pass, short decay.
fn bass(freq: f32) -> Box<dyn AudioUnit> {
    Box::new(
        ((sine_hz(freq) + saw_hz(freq) * 0.35) >> lowpass_hz(500.0, 0.7))
            * envelope(|t: f32| {
                let a = 0.005;
                if t < a {
                    t / a
                } else {
                    (-(t - a) * 4.0).exp()
                }
            })
            * 0.5,
    )
}

/// Lead: a **forward** voice — detuned saws + a square for body, through a brighter resonant
/// low-pass, with a longer sustain so it reads as a melody on top rather than background texture.
fn lead(freq: f32) -> Box<dyn AudioUnit> {
    Box::new(
        ((saw_hz(freq) + saw_hz(freq * 1.007) + saw_hz(freq * 0.993) + square_hz(freq) * 0.5)
            * 0.32
            >> lowpass_hz(3200.0, 1.6))
            * envelope(|t: f32| {
                let a = 0.012;
                if t < a {
                    t / a
                } else {
                    0.25 + 0.75 * (-(t - a) * 1.4).exp() // longer, more sustained tail
                }
            })
            * 0.85,
    )
}

/// Arp: a bright plucky counter-line (saw + square through a resonant low-pass, fast decay) — the
/// second melodic voice, an octave up, panned to dance opposite the lead.
fn arp(freq: f32) -> Box<dyn AudioUnit> {
    Box::new(
        ((saw_hz(freq) + square_hz(freq) * 0.6) * 0.5 >> lowpass_hz(3800.0, 1.2))
            * envelope(|t: f32| {
                let a = 0.004;
                if t < a {
                    t / a
                } else {
                    (-(t - a) * 9.0).exp()
                }
            })
            * 0.45,
    )
}

/// Equal-power pan gains for `pan` in [-1, 1] (-1 = hard left, 0 = centre, 1 = hard right).
fn pan_gains(pan: f32) -> (f32, f32) {
    let a = (pan.clamp(-1.0, 1.0) + 1.0) * (std::f32::consts::FRAC_PI_4); // 0..PI/2
    (a.cos(), a.sin())
}

/// Render a voice `node` into the interleaved-stereo `buf` at `start_t`s for `dur`s, scaled by
/// `amp` and panned by `pan`, with a 4 ms release fade so sustained voices don't click at cut-off.
fn render_into(
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

/// Render the triad as three voices panned across the field (wide chords), via `voice(freq)`.
fn chord_spread(
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

/// Spread reverb send: a mono sum of the stereo bed through 3 damped feedback combs per channel,
/// with slightly different delays L vs R → a wide, decorrelated room tail (dry excluded).
fn reverb_send(bed: &[f32], sr: f32) -> Vec<f32> {
    let frames = bed.len() / 2;
    let damp = 0.35_f32;
    // mono sum of the bed feeds the reverb.
    let mono: Vec<f32> = (0..frames)
        .map(|i| 0.5 * (bed[2 * i] + bed[2 * i + 1]))
        .collect();
    let comb = |delays: &[(f32, f32)]| -> Vec<f32> {
        let mut wet = vec![0f32; frames];
        for &(ds, fb) in delays {
            let d = (ds * sr) as usize;
            if d == 0 {
                continue;
            }
            let mut line = vec![0f32; frames];
            let mut lp = 0f32;
            for i in 0..frames {
                let fb_in = if i >= d { line[i - d] } else { 0.0 };
                lp += damp * (fb_in - lp);
                line[i] = mono[i] + fb * lp;
                wet[i] += fb * lp;
            }
        }
        wet
    };
    let wl = comb(&[(0.0297, 0.78), (0.0371, 0.80), (0.0411, 0.76)]);
    let wr = comb(&[(0.0319, 0.79), (0.0353, 0.77), (0.0431, 0.80)]);
    let mut out = vec![0f32; bed.len()];
    for i in 0..frames {
        out[2 * i] = wl[i];
        out[2 * i + 1] = wr[i];
    }
    out
}

/// Render the whole score to an interleaved-stereo buffer: voices panned into a "bed" (everything
/// but the kick), an arp counter-line in the energetic sections, sidechain pump under the kick, a
/// spread reverb send, the continuous sub, per-section fades × gain, soft clip.
pub fn synth_track(score: &Score) -> Track {
    use std::f32::consts::TAU;
    let sr = SAMPLE_RATE as f32;
    let total = (score.demo_len() * sr).ceil() as usize;
    let stereo = total * 2;
    let mut kickbuf = vec![0f32; stereo]; // sidechain source (never ducked)
    let mut bed = vec![0f32; stereo]; // everything else (pumped + reverbed)

    let kicks = score.hits(Inst::Kick);
    for &kt in &kicks {
        render_into(&mut kickbuf, kt, 0.45, 0.7, 0.0, kick());
        render_into(
            &mut bed,
            kt,
            0.55,
            0.5,
            0.0,
            bass(score.chord_at(kt).root * 0.5),
        );
    }
    for t in score.hits(Inst::Snare) {
        render_into(&mut bed, t, 0.4, 0.5, 0.0, snare());
    }
    for (i, t) in score.hits(Inst::Hat).into_iter().enumerate() {
        let pan = if i % 2 == 0 { 0.5 } else { -0.5 }; // hats dance across the field
        render_into(&mut bed, t, 0.12, 0.2, pan, hat());
    }
    for t in score.hits(Inst::Stab) {
        let m = score.levels(t).mids;
        chord_spread(
            &mut bed,
            t,
            0.5,
            0.10 + 0.10 * m,
            0.6,
            score.chord_at(t).triad(),
            stab,
        );
    }
    // lead: forward + centre.
    for (t, f) in score.lead_notes() {
        render_into(&mut bed, t, 0.6, 0.22, 0.0, lead(f));
    }
    // arp counter-line: a score note-lane (`<section>.arp` in assets/score.txt), panned alternately.
    for (i, (t, f)) in score.arp_notes().into_iter().enumerate() {
        let pan = if i % 2 == 0 { 0.55 } else { -0.55 };
        render_into(&mut bed, t, 0.2, 0.11, pan, arp(f));
    }
    // sustained pad: one chord per bar, spread wide (warmth/body).
    let bar = score.bar();
    let nbars = (score.demo_len() / bar).ceil() as usize;
    for b in 0..nbars {
        let t = b as f32 * bar;
        let m = score.levels(t).mids;
        chord_spread(
            &mut bed,
            t,
            bar,
            0.06 + 0.06 * m,
            0.7,
            score.chord_at(t).triad(),
            pad,
        );
    }
    // continuous sub-bass (centre) into the bed so it pumps with the sidechain.
    let dt = 1.0 / sr;
    for i in 0..total {
        let t = i as f32 * dt;
        let s = (TAU * 55.0 * t).sin() * (0.12 + 0.45 * score.levels(t).sub_bass);
        bed[2 * i] += s;
        bed[2 * i + 1] += s;
    }

    // sidechain pump: a fast dip right on each kick recovering over ~0.11s → the dance "breath".
    let mut duck = vec![1.0f32; total];
    let (depth, tau) = (0.55f32, 0.11f32);
    for &kt in &kicks {
        let k0 = (kt * sr) as usize;
        for j in 0..(0.34 * sr) as usize {
            let i = k0 + j;
            if i >= total {
                break;
            }
            let d = 1.0 - depth * (-(j as f32 / sr) / tau).exp();
            if d < duck[i] {
                duck[i] = d;
            }
        }
    }

    let wet = reverb_send(&bed, sr);

    // master: dry kick + pumped bed + spread reverb tail, per-section fades × gain_at, soft clip.
    let demo = score.demo_len();
    let mut buf = vec![0f32; stereo];
    for i in 0..total {
        let t = i as f32 * dt;
        let fade_in = (t / 1.5).clamp(0.0, 1.0);
        let fade_out = ((demo - t) / 2.0).clamp(0.0, 1.0);
        let g = fade_in * fade_out * score.gain_at(t);
        for c in 0..2 {
            let mix = kickbuf[2 * i + c] + bed[2 * i + c] * duck[i] + wet[2 * i + c] * 0.18;
            buf[2 * i + c] = (mix * g).tanh();
        }
    }
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
