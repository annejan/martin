//! Music score + section/beat clock — ported from Cinder's (Kristian Vlaardingerbroek, deFEEST)
//! `term-demo` (MIT, Outline 2026). Pure: a BPM→beat→bar grid, the section timeline
//! (Intro→Build→Drop→Breakdown→Climax→Outro), the drum/bass patterns, and the per-section
//! gain/envelope curves the synth (`audio.rs`) reads. martin also reads `section_at` / `BEAT` /
//! `BAR` to time the visual show to the music. (term-demo's terminal scene-cue consts are
//! dropped — martin renders gaussian splats, not ASCII scenes.)

pub const BPM: f32 = 140.0;
pub const BEAT: f32 = 60.0 / BPM;
pub const BAR: f32 = 4.0 * BEAT;

pub const DEMO_LEN: f32 = 54.0 * BAR;

pub const T_BUILD: f32 = 4.0 * BAR;
pub const T_DROP: f32 = 14.0 * BAR;
pub const T_BREAKDOWN: f32 = 24.0 * BAR;
pub const T_CLIMAX: f32 = 30.0 * BAR;
pub const T_OUTRO: f32 = 48.0 * BAR;

/// Crossfade window (seconds) for smoothing per-section gain/envelope steps at boundaries —
/// long enough to kill the click, short enough not to smear the musical transition.
pub const SECTION_FADE: f32 = 0.12;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Section {
    Intro,
    Build,
    Drop,
    Breakdown,
    Climax,
    Outro,
}

#[derive(Clone, Copy, Debug)]
pub struct Score {
    pub sub_bass: f32,
    pub mids: f32,
}

pub fn section_at(t: f32) -> Section {
    if t < T_BUILD {
        Section::Intro
    } else if t < T_DROP {
        Section::Build
    } else if t < T_BREAKDOWN {
        Section::Drop
    } else if t < T_CLIMAX {
        Section::Breakdown
    } else if t < T_OUTRO {
        Section::Climax
    } else {
        Section::Outro
    }
}

/// Start time (seconds) of the section active at `t`.
pub fn section_start(t: f32) -> f32 {
    match section_at(t) {
        Section::Intro => 0.0,
        Section::Build => T_BUILD,
        Section::Drop => T_DROP,
        Section::Breakdown => T_BREAKDOWN,
        Section::Climax => T_CLIMAX,
        Section::Outro => T_OUTRO,
    }
}

/// Crossfade a per-section value across its section boundary to remove the step-click: for the
/// first `SECTION_FADE` seconds of a section, lerp from the previous section's final value to
/// `f(t)`; elsewhere it is just `f(t)`. `f` is assumed continuous *within* a section, so this
/// only smooths the jump at the boundary.
pub fn smooth_section_boundary(t: f32, f: impl Fn(f32) -> f32) -> f32 {
    let b = section_start(t);
    let cur = f(t);
    if b > 0.0 && t - b < SECTION_FADE {
        let prev = f(b - 1e-3); // the previous section's value at the boundary
        let a = (t - b) / SECTION_FADE; // 0 → 1 across the window
        prev + (cur - prev) * a
    } else {
        cur
    }
}

// 16 slots per bar. Sentinel phase 255 = fill bar.
const F: bool = false;
const X: bool = true;

const KICK_INTRO: [bool; 16] = [F; 16];

const KICK_BUILD_P0: [bool; 16] = [X, F, F, F, F, F, F, F, F, F, F, F, X, F, F, F];
const KICK_BUILD_P1: [bool; 16] = [X, F, F, F, F, F, X, F, F, F, F, F, X, F, F, F];
const KICK_BUILD_FILL: [bool; 16] = [X, F, F, F, F, F, F, F, F, F, F, F, X, F, F, F];

const KICK_DROP_P0: [bool; 16] = [X, F, F, F, F, F, X, F, F, F, X, F, F, F, F, F];
const KICK_DROP_P1: [bool; 16] = [X, F, F, F, F, F, F, F, X, F, F, F, X, F, F, X];
const KICK_DROP_FILL: [bool; 16] = [X, F, F, F, F, F, F, F, F, F, F, F, X, F, F, F];

const KICK_BREAKDOWN_P0: [bool; 16] = [X, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F];
const KICK_BREAKDOWN_P1: [bool; 16] = [X, F, F, F, F, F, F, F, F, F, F, F, X, F, F, F];
const KICK_BREAKDOWN_FILL: [bool; 16] = [X, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F];

const KICK_CLIMAX_P0: [bool; 16] = [X, F, F, X, F, F, X, F, F, F, X, F, X, F, F, F];
const KICK_CLIMAX_P1: [bool; 16] = [X, F, F, F, X, F, F, X, F, X, F, F, X, F, F, X];
const KICK_CLIMAX_P2: [bool; 16] = [X, F, X, F, X, F, X, F, X, F, X, F, X, F, X, F];
const KICK_CLIMAX_FILL: [bool; 16] = [X, F, F, F, F, F, F, F, F, F, F, F, X, F, F, F];

const KICK_OUTRO_P0: [bool; 16] = [X, F, F, F, F, F, F, F, X, F, F, F, F, F, F, F];
const KICK_OUTRO_FILL: [bool; 16] = [X, F, F, F, F, F, F, F, F, F, F, F, F, F, F, F];

// Every drumming phase fires slot 0, so scene cuts on bar boundaries always land on a kick.
const _: () = assert!(
    KICK_BUILD_P0[0]
        && KICK_BUILD_P1[0]
        && KICK_BUILD_FILL[0]
        && KICK_DROP_P0[0]
        && KICK_DROP_P1[0]
        && KICK_DROP_FILL[0]
        && KICK_BREAKDOWN_P0[0]
        && KICK_BREAKDOWN_P1[0]
        && KICK_BREAKDOWN_FILL[0]
        && KICK_CLIMAX_P0[0]
        && KICK_CLIMAX_P1[0]
        && KICK_CLIMAX_P2[0]
        && KICK_CLIMAX_FILL[0]
        && KICK_OUTRO_P0[0]
        && KICK_OUTRO_FILL[0],
);

fn section_layout(s: Section) -> (u32, &'static [u32], bool) {
    match s {
        Section::Intro => (0, &[4], false),
        Section::Build => (4, &[4, 5], true),
        Section::Drop => (14, &[4, 5], true),
        Section::Breakdown => (24, &[3, 2], true),
        Section::Climax => (30, &[6, 6, 5], true),
        Section::Outro => (48, &[5], true),
    }
}

pub fn bar_idx_at(t: f32) -> usize {
    (abs_slot(t).max(0) / 16) as usize
}

pub fn section_phase(t: f32) -> u8 {
    let s = section_at(t);
    let (start_bar, phase_bars, has_fill) = section_layout(s);
    let into = (bar_idx_at(t) as i64 - start_bar as i64).max(0) as u32;
    if has_fill {
        let total: u32 = phase_bars.iter().sum::<u32>() + 1;
        if into >= total - 1 {
            return 255;
        }
    }
    let mut acc = 0;
    for (i, &p) in phase_bars.iter().enumerate() {
        acc += p;
        if into < acc {
            return i as u8;
        }
    }
    (phase_bars.len() - 1) as u8
}

pub fn kick_pattern_at(t: f32) -> &'static [bool; 16] {
    let s = section_at(t);
    let phase = section_phase(t);
    match (s, phase) {
        (Section::Intro, _) => &KICK_INTRO,

        (Section::Build, 0) => &KICK_BUILD_P0,
        (Section::Build, 1) => &KICK_BUILD_P1,
        (Section::Build, 255) => &KICK_BUILD_FILL,
        (Section::Build, _) => &KICK_BUILD_P1,

        (Section::Drop, 0) => &KICK_DROP_P0,
        (Section::Drop, 1) => &KICK_DROP_P1,
        (Section::Drop, 255) => &KICK_DROP_FILL,
        (Section::Drop, _) => &KICK_DROP_P1,

        (Section::Breakdown, 0) => &KICK_BREAKDOWN_P0,
        (Section::Breakdown, 1) => &KICK_BREAKDOWN_P1,
        (Section::Breakdown, 255) => &KICK_BREAKDOWN_FILL,
        (Section::Breakdown, _) => &KICK_BREAKDOWN_P1,

        (Section::Climax, 0) => &KICK_CLIMAX_P0,
        (Section::Climax, 1) => &KICK_CLIMAX_P1,
        (Section::Climax, 2) => &KICK_CLIMAX_P2,
        (Section::Climax, 255) => &KICK_CLIMAX_FILL,
        (Section::Climax, _) => &KICK_CLIMAX_P2,

        (Section::Outro, 0) => &KICK_OUTRO_P0,
        (Section::Outro, 255) => &KICK_OUTRO_FILL,
        (Section::Outro, _) => &KICK_OUTRO_P0,
    }
}

fn abs_slot(t: f32) -> i64 {
    let slot_len = BEAT / 4.0;
    let eps = slot_len * 1e-3;
    ((t + eps) / slot_len).floor() as i64
}

pub fn last_kick_time(t: f32) -> Option<f32> {
    if t < 0.0 {
        return None;
    }
    let slot_len = BEAT / 4.0;
    let mut slot = abs_slot(t);
    if slot >= 0 && (slot as f32) * slot_len > t {
        slot -= 1;
    }
    for _ in 0..(16 * 4) {
        if slot < 0 {
            return None;
        }
        let kt = slot as f32 * slot_len;
        let pat = kick_pattern_at(kt);
        let s = (slot.rem_euclid(16)) as usize;
        if pat[s] {
            return Some(kt);
        }
        slot -= 1;
    }
    None
}

fn sub_bass_depth(t: f32) -> f32 {
    match section_at(t) {
        Section::Intro => 0.25,
        Section::Build => {
            let p = ((t - T_BUILD) / (T_DROP - T_BUILD)).clamp(0.0, 1.0);
            0.25 + 0.55 * p
        }
        Section::Drop => 1.0,
        Section::Breakdown => 0.15,
        Section::Climax => 0.9,
        Section::Outro => 0.4,
    }
}

fn sub_bass_envelope(t: f32) -> f32 {
    use std::f32::consts::TAU;
    let breath = (t / (2.0 * BEAT) * TAU).sin() * 0.5 + 0.5;
    // crossfade the per-section depth at boundaries so the sub doesn't jump (e.g. Drop 1.0 →
    // Breakdown 0.15) with an audible step.
    let depth = smooth_section_boundary(t, sub_bass_depth);
    (breath * depth).clamp(0.0, 1.0)
}

fn mids_depth(t: f32) -> f32 {
    match section_at(t) {
        Section::Intro => 0.5,
        Section::Build => 0.7,
        Section::Drop => 0.9,
        Section::Breakdown => 0.6,
        Section::Climax => 1.0,
        Section::Outro => 0.45,
    }
}

fn mids_envelope(t: f32) -> f32 {
    use std::f32::consts::TAU;
    let swell = (t / (8.0 * BEAT) * TAU).sin() * 0.5 + 0.5;
    let depth = smooth_section_boundary(t, mids_depth);
    (swell * depth).clamp(0.0, 1.0)
}

pub fn sample(t: f32) -> Score {
    Score {
        sub_bass: sub_bass_envelope(t),
        mids: mids_envelope(t),
    }
}
