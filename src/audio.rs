//! Synth — the *instrument* (voices + DSP) for the placeholder track. The *score* it plays (tempo,
//! sections, drum patterns, chords, melody, dynamics) is data in `score.rs` (`assets/score.txt`).
//! Voices are FunDSP graphs (filtered/enveloped oscillators) — far richer than bare sines — built
//! per hit/note and rendered into one mono buffer; `score.rs` supplies all the timing + dynamics.
//! The whole track renders offline; martin plays it live (bevy_audio) and/or writes a WAV that
//! ffmpeg muxes onto recorded frames. (Still a placeholder — the real track comes from Cinder.)

use std::sync::Arc;

use fundsp::prelude32::*;

use crate::score::{Inst, Score};

pub const SAMPLE_RATE: u32 = 44_100;

#[derive(Clone)]
pub struct Track {
    samples: Arc<Vec<f32>>,
}

impl Track {
    pub fn len(&self) -> usize {
        self.samples.len()
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

/// Stab: the chord triad as saws through a low-pass, with a plucky attack.
fn stab(tri: [f32; 3]) -> Box<dyn AudioUnit> {
    Box::new(
        ((saw_hz(tri[0]) + saw_hz(tri[1]) + saw_hz(tri[2])) >> lowpass_hz(1600.0, 0.8))
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

/// Pad: the triad an octave down through a soft low-pass, slow swell — warmth/body under it all.
fn pad(tri: [f32; 3]) -> Box<dyn AudioUnit> {
    Box::new(
        ((saw_hz(tri[0] * 0.5) + saw_hz(tri[1] * 0.5) + saw_hz(tri[2] * 0.5))
            >> lowpass_hz(900.0, 0.6))
            * envelope(|t: f32| (t * 2.0).min(1.0))
            * 0.25,
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

/// Lead: a filtered saw — mellow, sits under the groove.
fn lead(freq: f32) -> Box<dyn AudioUnit> {
    Box::new(
        (saw_hz(freq) >> lowpass_hz(2200.0, 0.9))
            * envelope(|t: f32| {
                let a = 0.02;
                if t < a {
                    t / a
                } else {
                    (-(t - a) * 2.2).exp()
                }
            })
            * 0.5,
    )
}

/// Render a voice `node` into `buf` starting at `start_t` seconds for `dur` seconds, scaled by
/// `amp`, with a 4 ms release fade so sustained voices don't click at their cut-off.
fn render_into(buf: &mut [f32], start_t: f32, dur: f32, amp: f32, mut node: Box<dyn AudioUnit>) {
    let sr = SAMPLE_RATE as f32;
    node.set_sample_rate(SAMPLE_RATE as f64);
    node.reset();
    let start = (start_t * sr) as usize;
    let n = (dur * sr) as usize;
    let rel = (0.004 * sr) as usize;
    for i in 0..n {
        let idx = start + i;
        if idx >= buf.len() {
            break;
        }
        let fade = if n > rel && i >= n - rel {
            (n - i) as f32 / rel as f32
        } else {
            1.0
        };
        buf[idx] += node.get_mono() * amp * fade;
    }
}

/// Render the whole score to a mono buffer: build a FunDSP voice at every drum hit / lead note /
/// per-bar chord (timing + chord + dynamics all from `score`), then add the continuous sub and the
/// per-section master gain (fades + `gain_at`) and soft-clip.
pub fn synth_track(score: &Score) -> Track {
    use std::f32::consts::TAU;
    let sr = SAMPLE_RATE as f32;
    let total = (score.demo_len() * sr).ceil() as usize;
    let mut buf = vec![0f32; total];

    for kt in score.hits(Inst::Kick) {
        render_into(&mut buf, kt, 0.45, 0.7, kick());
        render_into(&mut buf, kt, 0.55, 0.5, bass(score.chord_at(kt).root * 0.5));
    }
    for t in score.hits(Inst::Snare) {
        render_into(&mut buf, t, 0.4, 0.5, snare());
    }
    for t in score.hits(Inst::Hat) {
        render_into(&mut buf, t, 0.12, 0.2, hat());
    }
    for t in score.hits(Inst::Stab) {
        let m = score.levels(t).mids;
        render_into(
            &mut buf,
            t,
            0.5,
            0.10 + 0.10 * m,
            stab(score.chord_at(t).triad()),
        );
    }
    for (t, f) in score.lead_notes() {
        render_into(&mut buf, t, 0.6, 0.13, lead(f));
    }
    // sustained pad: one chord per bar (warmth/body)
    let bar = score.bar();
    let nbars = (score.demo_len() / bar).ceil() as usize;
    for b in 0..nbars {
        let t = b as f32 * bar;
        let m = score.levels(t).mids;
        render_into(
            &mut buf,
            t,
            bar,
            0.06 + 0.06 * m,
            pad(score.chord_at(t).triad()),
        );
    }

    // continuous sub-bass + per-section master gain (fade-in/out × gain_at) + soft clip.
    let dt = 1.0 / sr;
    let demo = score.demo_len();
    for (i, s) in buf.iter_mut().enumerate() {
        let t = i as f32 * dt;
        let lv = score.levels(t);
        let sub = (TAU * 55.0 * t).sin() * (0.12 + 0.45 * lv.sub_bass);
        let fade_in = (t / 1.5).clamp(0.0, 1.0);
        let fade_out = ((demo - t) / 2.0).clamp(0.0, 1.0);
        let g = fade_in * fade_out * score.gain_at(t);
        *s = ((*s + sub) * g).tanh();
    }
    Track {
        samples: Arc::new(buf),
    }
}

/// Encode the track as a 16-bit PCM mono WAV (`SAMPLE_RATE`) into a byte buffer — hand-rolled RIFF
/// header, no audio dependency. Reused for the on-disk WAV (`write_wav`) and for in-app live
/// playback (bevy_audio decodes these bytes).
pub fn encode_wav(track: &Track) -> Vec<u8> {
    let data_bytes = (track.samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data_bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes()); // PCM fmt chunk size
    out.extend_from_slice(&1u16.to_le_bytes()); // format = PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // channels = mono
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes()); // sample rate
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes()); // byte rate (rate * block align)
    out.extend_from_slice(&2u16.to_le_bytes()); // block align (mono * 2 bytes)
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
