//! Procedural synth — ported from Cinder's (Kristian Vlaardingerbroek, deFEEST) `term-demo`
//! (MIT, released at Outline 2026). The whole track is rendered offline to a sample buffer;
//! martin writes it to a WAV (`write_wav`) and ffmpeg muxes it onto the recorded video. The
//! section/beat clock that shapes it lives in `score.rs` and also drives the visual timeline.

use std::sync::Arc;

use crate::score::{
    self, section_phase, Section, BEAT, DEMO_LEN, T_BREAKDOWN, T_BUILD, T_CLIMAX, T_DROP, T_OUTRO,
};

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

// 16 slots per bar (16th notes).
const F: bool = false;
const X: bool = true;

const SNARE_INTRO: [bool; 16] = [F; 16];
const HAT_INTRO: [bool; 16] = [F; 16];
const STAB_INTRO: [bool; 16] = [F; 16];

const SNARE_BUILD_P0: [bool; 16] = [F; 16];
const SNARE_BUILD_P1: [bool; 16] = [F, F, F, F, X, F, F, F, F, F, F, F, X, F, F, F];
const SNARE_BUILD_FILL: [bool; 16] = [F, F, F, F, X, F, X, F, X, F, X, F, X, X, X, X];

const HAT_BUILD_P0: [bool; 16] = [X, F, F, F, X, F, F, F, X, F, F, F, X, F, F, F];
const HAT_BUILD_P1: [bool; 16] = [X, F, X, F, X, F, X, F, X, F, X, F, X, F, X, F];
const HAT_BUILD_FILL: [bool; 16] = [X; 16];

const STAB_BUILD_P0: [bool; 16] = [F; 16];
const STAB_BUILD_P1: [bool; 16] = [F, F, F, F, F, F, X, F, F, F, F, F, F, F, F, F];
const STAB_BUILD_FILL: [bool; 16] = [F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, X];

const SNARE_DROP_P0: [bool; 16] = [F, F, F, F, X, F, F, X, F, F, F, F, X, F, F, X];
const SNARE_DROP_P1: [bool; 16] = [F, F, X, F, F, F, F, F, F, F, X, F, X, F, F, F];
const SNARE_DROP_FILL: [bool; 16] = [F, F, F, F, X, F, X, X, F, X, X, X, X, X, X, X];

const HAT_DROP_P0: [bool; 16] = [X; 16];
const HAT_DROP_P1: [bool; 16] = [X; 16];
const HAT_DROP_FILL: [bool; 16] = [X; 16];

const STAB_DROP_P0: [bool; 16] = [F, F, F, F, F, F, X, F, F, F, F, F, F, F, X, F];
const STAB_DROP_P1: [bool; 16] = [F, F, F, F, X, F, F, F, F, F, F, F, X, F, F, F];
const STAB_DROP_FILL: [bool; 16] = [F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, X];

const SNARE_BREAKDOWN_P0: [bool; 16] = [F; 16];
const SNARE_BREAKDOWN_P1: [bool; 16] = [F, F, F, F, F, F, F, F, F, F, F, F, X, F, F, F];
const SNARE_BREAKDOWN_FILL: [bool; 16] = [F, F, X, X, F, X, X, X, F, X, X, X, X, X, X, X];

const HAT_BREAKDOWN_P0: [bool; 16] = [F; 16];
const HAT_BREAKDOWN_P1: [bool; 16] = [X, F, F, F, X, F, F, F, X, F, F, F, X, F, F, F];
const HAT_BREAKDOWN_FILL: [bool; 16] = [X; 16];

const STAB_BREAKDOWN_P0: [bool; 16] = [F, F, F, F, F, F, F, F, X, F, F, F, F, F, F, F];
const STAB_BREAKDOWN_P1: [bool; 16] = [F; 16];
const STAB_BREAKDOWN_FILL: [bool; 16] = [F; 16];

const SNARE_CLIMAX_P0: [bool; 16] = [F, F, F, F, X, F, F, X, F, F, F, F, X, F, X, X];
const SNARE_CLIMAX_P1: [bool; 16] = [F, F, X, F, F, F, X, F, F, X, F, F, F, F, X, X];
const SNARE_CLIMAX_P2: [bool; 16] = [X, F, X, F, X, F, X, F, X, F, X, F, X, F, X, X];
const SNARE_CLIMAX_FILL: [bool; 16] = [F, F, F, F, X, X, X, X, X, X, X, X, X, X, X, X];

const HAT_CLIMAX_P0: [bool; 16] = [X; 16];
const HAT_CLIMAX_P1: [bool; 16] = [X; 16];
const HAT_CLIMAX_P2: [bool; 16] = [X; 16];
const HAT_CLIMAX_FILL: [bool; 16] = [X; 16];

const STAB_CLIMAX_P0: [bool; 16] = [F, F, F, F, F, F, X, F, F, F, X, F, F, F, X, F];
const STAB_CLIMAX_P1: [bool; 16] = [F, X, F, F, F, F, X, F, F, X, F, F, F, F, X, F];
const STAB_CLIMAX_P2: [bool; 16] = [X, F, F, X, F, X, X, F, X, F, X, F, X, F, X, X];
const STAB_CLIMAX_FILL: [bool; 16] = [F, F, F, F, F, F, F, F, F, F, F, F, F, F, F, X];

const SNARE_OUTRO_P0: [bool; 16] = [F, F, F, F, X, F, F, F, F, F, F, F, X, F, F, F];
const SNARE_OUTRO_FILL: [bool; 16] = [F, F, F, F, X, F, F, F, F, F, F, F, F, F, F, F];

const HAT_OUTRO_P0: [bool; 16] = [X, F, X, F, X, F, X, F, X, F, X, F, X, F, X, F];
const HAT_OUTRO_FILL: [bool; 16] = [F; 16];

const STAB_OUTRO_P0: [bool; 16] = [F, F, F, F, F, F, X, F, F, F, F, F, F, F, F, F];
const STAB_OUTRO_FILL: [bool; 16] = [F, F, F, F, F, F, X, F, F, F, F, F, F, F, F, F];

fn snare_pattern_at(t: f32) -> &'static [bool; 16] {
    let s = score::section_at(t);
    let phase = section_phase(t);
    match (s, phase) {
        (Section::Intro, _) => &SNARE_INTRO,

        (Section::Build, 0) => &SNARE_BUILD_P0,
        (Section::Build, 1) => &SNARE_BUILD_P1,
        (Section::Build, 255) => &SNARE_BUILD_FILL,
        (Section::Build, _) => &SNARE_BUILD_P1,

        (Section::Drop, 0) => &SNARE_DROP_P0,
        (Section::Drop, 1) => &SNARE_DROP_P1,
        (Section::Drop, 255) => &SNARE_DROP_FILL,
        (Section::Drop, _) => &SNARE_DROP_P1,

        (Section::Breakdown, 0) => &SNARE_BREAKDOWN_P0,
        (Section::Breakdown, 1) => &SNARE_BREAKDOWN_P1,
        (Section::Breakdown, 255) => &SNARE_BREAKDOWN_FILL,
        (Section::Breakdown, _) => &SNARE_BREAKDOWN_P1,

        (Section::Climax, 0) => &SNARE_CLIMAX_P0,
        (Section::Climax, 1) => &SNARE_CLIMAX_P1,
        (Section::Climax, 2) => &SNARE_CLIMAX_P2,
        (Section::Climax, 255) => &SNARE_CLIMAX_FILL,
        (Section::Climax, _) => &SNARE_CLIMAX_P2,

        (Section::Outro, 0) => &SNARE_OUTRO_P0,
        (Section::Outro, 255) => &SNARE_OUTRO_FILL,
        (Section::Outro, _) => &SNARE_OUTRO_P0,
    }
}

fn hat_pattern_at(t: f32) -> &'static [bool; 16] {
    let s = score::section_at(t);
    let phase = section_phase(t);
    match (s, phase) {
        (Section::Intro, _) => &HAT_INTRO,

        (Section::Build, 0) => &HAT_BUILD_P0,
        (Section::Build, 1) => &HAT_BUILD_P1,
        (Section::Build, 255) => &HAT_BUILD_FILL,
        (Section::Build, _) => &HAT_BUILD_P1,

        (Section::Drop, 0) => &HAT_DROP_P0,
        (Section::Drop, 1) => &HAT_DROP_P1,
        (Section::Drop, 255) => &HAT_DROP_FILL,
        (Section::Drop, _) => &HAT_DROP_P1,

        (Section::Breakdown, 0) => &HAT_BREAKDOWN_P0,
        (Section::Breakdown, 1) => &HAT_BREAKDOWN_P1,
        (Section::Breakdown, 255) => &HAT_BREAKDOWN_FILL,
        (Section::Breakdown, _) => &HAT_BREAKDOWN_P1,

        (Section::Climax, 0) => &HAT_CLIMAX_P0,
        (Section::Climax, 1) => &HAT_CLIMAX_P1,
        (Section::Climax, 2) => &HAT_CLIMAX_P2,
        (Section::Climax, 255) => &HAT_CLIMAX_FILL,
        (Section::Climax, _) => &HAT_CLIMAX_P2,

        (Section::Outro, 0) => &HAT_OUTRO_P0,
        (Section::Outro, 255) => &HAT_OUTRO_FILL,
        (Section::Outro, _) => &HAT_OUTRO_P0,
    }
}

fn stab_pattern_at(t: f32) -> &'static [bool; 16] {
    let s = score::section_at(t);
    let phase = section_phase(t);
    match (s, phase) {
        (Section::Intro, _) => &STAB_INTRO,

        (Section::Build, 0) => &STAB_BUILD_P0,
        (Section::Build, 1) => &STAB_BUILD_P1,
        (Section::Build, 255) => &STAB_BUILD_FILL,
        (Section::Build, _) => &STAB_BUILD_P1,

        (Section::Drop, 0) => &STAB_DROP_P0,
        (Section::Drop, 1) => &STAB_DROP_P1,
        (Section::Drop, 255) => &STAB_DROP_FILL,
        (Section::Drop, _) => &STAB_DROP_P1,

        (Section::Breakdown, 0) => &STAB_BREAKDOWN_P0,
        (Section::Breakdown, 1) => &STAB_BREAKDOWN_P1,
        (Section::Breakdown, 255) => &STAB_BREAKDOWN_FILL,
        (Section::Breakdown, _) => &STAB_BREAKDOWN_P1,

        (Section::Climax, 0) => &STAB_CLIMAX_P0,
        (Section::Climax, 1) => &STAB_CLIMAX_P1,
        (Section::Climax, 2) => &STAB_CLIMAX_P2,
        (Section::Climax, 255) => &STAB_CLIMAX_FILL,
        (Section::Climax, _) => &STAB_CLIMAX_P2,

        (Section::Outro, 0) => &STAB_OUTRO_P0,
        (Section::Outro, 255) => &STAB_OUTRO_FILL,
        (Section::Outro, _) => &STAB_OUTRO_P0,
    }
}

fn last_pattern_hit(t: f32, pattern_for: fn(f32) -> &'static [bool; 16]) -> Option<f32> {
    if t < 0.0 {
        return None;
    }
    let slot_len = BEAT / 4.0;
    let eps = slot_len * 1e-3;
    let mut slot = ((t + eps) / slot_len).floor() as i64;
    if slot >= 0 && (slot as f32) * slot_len > t {
        slot -= 1;
    }
    for _ in 0..(16 * 4) {
        if slot < 0 {
            return None;
        }
        let kt = slot as f32 * slot_len;
        let pat = pattern_for(kt);
        let s = slot.rem_euclid(16) as usize;
        if pat[s] {
            return Some(kt);
        }
        slot -= 1;
    }
    None
}

// Am triad: A2, C3, E3.
const AM_CHORD: [f32; 3] = [110.00, 130.81, 164.81];

// A2 / E2 / G2, rotated per bar.
const BASS_NOTES: [f32; 3] = [110.0, 82.41, 98.00];

fn bass_note_for(hit_time: f32) -> f32 {
    let bar = (4.0 * BEAT).max(f32::MIN_POSITIVE);
    let bar_idx = (hit_time / bar).floor().max(0.0) as usize;
    BASS_NOTES[bar_idx % BASS_NOTES.len()]
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

fn synth_sample(t: f32) -> f32 {
    use std::f32::consts::TAU;
    let s = score::sample(t);

    let sub_freq = 55.0;
    let sub = (t * sub_freq * TAU).sin() * (0.12 + 0.45 * s.sub_bass);

    let kick = score::last_kick_time(t).map_or(0.0, |kt| {
        let dt = t - kt;
        let pitch = 45.0 + 75.0 * (-dt / 0.03).exp();
        let env = (-dt / 0.12).exp();
        (dt * pitch * TAU).sin() * env * 0.55
    });

    let snare = last_pattern_hit(t, snare_pattern_at).map_or(0.0, |kt| {
        let dt = t - kt;
        if dt > 0.5 {
            0.0
        } else {
            snare_voice(t, dt) * 0.35
        }
    });

    let hat = last_pattern_hit(t, hat_pattern_at).map_or(0.0, |kt| {
        let dt = t - kt;
        if dt > 0.15 {
            0.0
        } else {
            hat_voice(t, dt) * 0.08
        }
    });

    let stab = last_pattern_hit(t, stab_pattern_at).map_or(0.0, |kt| {
        let dt = t - kt;
        if dt > 0.4 {
            0.0
        } else {
            let env = stab_envelope(dt);
            (pad_voice(t, AM_CHORD[0]) + pad_voice(t, AM_CHORD[1]) + pad_voice(t, AM_CHORD[2]))
                * env
                * (0.03 + 0.05 * s.mids)
        }
    });

    let bass = score::last_kick_time(t).map_or(0.0, |kt| {
        let dt = t - kt;
        // Let the bass ring out, then fade its tail to silence — instead of hard-zeroing it
        // while still ~14% audible, which was the abrupt "bass cutoff" click.
        let cut = 0.9;
        let fade = 0.08;
        if dt > cut {
            0.0
        } else {
            let freq = bass_note_for(kt);
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

    let master = {
        let fade_in = (t / 1.5).clamp(0.0, 1.0);
        let fade_out = ((DEMO_LEN - t) / 2.0).clamp(0.0, 1.0);
        // crossfade the per-section master gain at boundaries — a hard step here was the main
        // "rough volume change" (a click / sudden level jump between sections).
        let section_gain = score::smooth_section_boundary(t, |tt| match score::section_at(tt) {
            Section::Intro => 0.5,
            Section::Build => 0.85,
            Section::Drop => 1.0,
            Section::Breakdown => 0.6,
            Section::Climax => 1.0,
            Section::Outro => 0.7,
        });
        fade_in * fade_out * section_gain
    };

    let mix = (sub + kick + snare + hat + stab + bass) * master;
    mix.tanh()
}

pub fn synth_track() -> Track {
    let total = (DEMO_LEN * SAMPLE_RATE as f32).ceil() as usize;
    let dt = 1.0 / SAMPLE_RATE as f32;
    let samples: Vec<f32> = (0..total).map(|i| synth_sample(i as f32 * dt)).collect();
    debug_assert_eq!(score::section_at(0.0), Section::Intro);
    debug_assert_eq!(score::section_at(T_BUILD), Section::Build);
    debug_assert_eq!(score::section_at(T_DROP), Section::Drop);
    debug_assert_eq!(score::section_at(T_BREAKDOWN), Section::Breakdown);
    debug_assert_eq!(score::section_at(T_CLIMAX), Section::Climax);
    debug_assert_eq!(score::section_at(T_OUTRO), Section::Outro);
    let track = Track {
        samples: Arc::new(samples),
    };
    debug_assert_eq!(track.len(), total);
    track
}

/// Write the track as a 16-bit PCM mono WAV at `SAMPLE_RATE`, so ffmpeg can mux it onto the
/// recorded frames. Hand-rolled RIFF header — no audio dependency needed.
pub fn write_wav(track: &Track, path: &str) -> std::io::Result<()> {
    use std::io::Write;
    let data_bytes = (track.samples.len() * 2) as u32;
    let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
    out.write_all(b"RIFF")?;
    out.write_all(&(36 + data_bytes).to_le_bytes())?;
    out.write_all(b"WAVE")?;
    out.write_all(b"fmt ")?;
    out.write_all(&16u32.to_le_bytes())?; // PCM fmt chunk size
    out.write_all(&1u16.to_le_bytes())?; // format = PCM
    out.write_all(&1u16.to_le_bytes())?; // channels = mono
    out.write_all(&SAMPLE_RATE.to_le_bytes())?; // sample rate
    out.write_all(&(SAMPLE_RATE * 2).to_le_bytes())?; // byte rate (rate * block align)
    out.write_all(&2u16.to_le_bytes())?; // block align (mono * 2 bytes)
    out.write_all(&16u16.to_le_bytes())?; // bits per sample
    out.write_all(b"data")?;
    out.write_all(&data_bytes.to_le_bytes())?;
    for &s in track.samples.iter() {
        out.write_all(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())?;
    }
    out.flush()
}
