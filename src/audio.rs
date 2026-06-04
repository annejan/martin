//! Procedural synth — the *instrument* (voices + DSP), ported from Cinder's (Kristian
//! Vlaardingerbroek, deFEEST) `term-demo` (MIT, Outline 2026). The *score* it plays (tempo,
//! sections, patterns, dynamics) is data in `score.rs` — `Score::builtin()` or a `MARTIN_SCORE`
//! file. The whole track renders offline to a sample buffer; martin plays it live (bevy_audio)
//! and/or writes a WAV (`write_wav`) for ffmpeg to mux onto recorded frames.

use std::sync::Arc;

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

fn pad_voice(t: f32, freq: f32) -> f32 {
    use std::f32::consts::TAU;
    let a = (t * freq * TAU).sin();
    let b = (t * freq * 1.003 * TAU).sin();
    (a + b) * 0.5
}

fn bass_voice(t: f32, freq: f32) -> f32 {
    use std::f32::consts::TAU;
    let f1 = (t * freq * TAU).sin();
    let f3 = (t * 3.0 * freq * TAU).sin();
    let f5 = (t * 5.0 * freq * TAU).sin();
    f1 + 0.4 * f3 + 0.15 * f5
}

fn noise_hash(idx: u32, salt: u32) -> f32 {
    let n = (idx ^ salt).wrapping_mul(2654435761);
    (n as f32 / u32::MAX as f32) * 2.0 - 1.0
}

fn noise_sample_lowpassed(t: f32, salt: u32) -> f32 {
    let idx = (t as f64 * SAMPLE_RATE as f64) as u32;
    let n0 = noise_hash(idx, salt);
    let n1 = noise_hash(idx.saturating_sub(1), salt);
    let n2 = noise_hash(idx.saturating_sub(2), salt);
    let n3 = noise_hash(idx.saturating_sub(3), salt);
    (n0 + n1 + n2 + n3) * 0.25
}

fn snare_voice(t: f32, dt: f32) -> f32 {
    use std::f32::consts::TAU;
    let tone = (dt * 140.0 * TAU).sin();
    let noise = noise_sample_lowpassed(t, 0x5C5A_E9F1);
    let env_tone = (-dt / 0.05).exp();
    let env_noise = (-dt / 0.09).exp();
    tone * env_tone * 0.5 + noise * env_noise * 0.6
}

fn hat_voice(t: f32, dt: f32) -> f32 {
    let noise = noise_sample_lowpassed(t, 0xA17B_C9C3);
    let env = (-dt / 0.025).exp();
    noise * env
}

fn stab_envelope(dt: f32) -> f32 {
    if dt < 0.0 {
        return 0.0;
    }
    let attack: f32 = 0.030;
    if dt < attack {
        (dt / attack).clamp(0.0, 1.0)
    } else {
        (-(dt - attack) / 0.120).exp().clamp(0.0, 1.0)
    }
}

/// Bright saw-ish lead voice (first few harmonics) with a fast attack + exponential decay.
fn lead_voice(t: f32, freq: f32, dt: f32) -> f32 {
    use std::f32::consts::TAU;
    let osc = (t * freq * TAU).sin()
        + 0.5 * (t * 2.0 * freq * TAU).sin()
        + 0.33 * (t * 3.0 * freq * TAU).sin()
        + 0.25 * (t * 4.0 * freq * TAU).sin();
    let attack = 0.008;
    let env = if dt < attack {
        dt / attack
    } else {
        (-(dt - attack) / 0.45).exp()
    };
    osc * env
}

/// One mono sample of the mix at time `t`, reading the patterns + dynamics from `score`.
fn synth_sample(score: &Score, t: f32) -> f32 {
    use std::f32::consts::TAU;
    let lv = score.levels(t);

    let sub_freq = 55.0;
    let sub = (t * sub_freq * TAU).sin() * (0.12 + 0.45 * lv.sub_bass);

    let kick = score.last_hit(Inst::Kick, t).map_or(0.0, |kt| {
        let dt = t - kt;
        let pitch = 45.0 + 75.0 * (-dt / 0.03).exp();
        let env = (-dt / 0.12).exp();
        (dt * pitch * TAU).sin() * env * 0.55
    });

    let snare = score.last_hit(Inst::Snare, t).map_or(0.0, |kt| {
        let dt = t - kt;
        if dt > 0.5 {
            0.0
        } else {
            snare_voice(t, dt) * 0.35
        }
    });

    let hat = score.last_hit(Inst::Hat, t).map_or(0.0, |kt| {
        let dt = t - kt;
        if dt > 0.15 {
            0.0
        } else {
            hat_voice(t, dt) * 0.08
        }
    });

    let stab = score.last_hit(Inst::Stab, t).map_or(0.0, |kt| {
        let dt = t - kt;
        if dt > 0.4 {
            0.0
        } else {
            let env = stab_envelope(dt);
            let tri = score.chord_at(kt).triad();
            (pad_voice(t, tri[0]) + pad_voice(t, tri[1]) + pad_voice(t, tri[2]))
                * env
                * (0.03 + 0.05 * lv.mids)
        }
    });

    let bass = score.last_hit(Inst::Kick, t).map_or(0.0, |kt| {
        let dt = t - kt;
        // Let the bass ring out, then fade its tail to silence — instead of hard-zeroing it while
        // still ~14% audible, which was the abrupt "bass cutoff" click.
        let cut = 0.9;
        let fade = 0.08;
        if dt > cut {
            0.0
        } else {
            let freq = score.chord_at(kt).root * 0.5; // chord root, an octave down
            let attack: f32 = 0.010;
            let env = if dt < attack {
                (dt / attack).clamp(0.0, 1.0)
            } else {
                (-(dt - attack) / 0.25).exp()
            };
            let tail = ((cut - dt) / fade).clamp(0.0, 1.0); // smooth release to zero at `cut`
            bass_voice(t, freq) * env * tail * 0.18
        }
    });

    // melody: the most recent lead note, played by the lead voice (silent in lead-less sections).
    let lead = score.last_lead(t).map_or(0.0, |(freq, lt)| {
        let dt = t - lt;
        if dt > 1.5 {
            0.0
        } else {
            lead_voice(t, freq, dt) * 0.16
        }
    });

    let master = {
        let fade_in = (t / 1.5).clamp(0.0, 1.0);
        let fade_out = ((score.demo_len() - t) / 2.0).clamp(0.0, 1.0);
        fade_in * fade_out * score.gain_at(t)
    };

    ((sub + kick + snare + hat + stab + bass + lead) * master).tanh()
}

/// Render the whole score to a mono sample buffer.
pub fn synth_track(score: &Score) -> Track {
    let total = (score.demo_len() * SAMPLE_RATE as f32).ceil() as usize;
    let dt = 1.0 / SAMPLE_RATE as f32;
    let samples: Vec<f32> = (0..total)
        .map(|i| synth_sample(score, i as f32 * dt))
        .collect();
    Track {
        samples: Arc::new(samples),
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
