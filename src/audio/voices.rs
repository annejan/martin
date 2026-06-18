//! The *instrument* layer: each voice is a FunDSP graph (a 0-input → 1-output unit) the renderer
//! plays into the bed. The distortion-heavy ones (saw/tanh stacks) optionally 2× oversample via the
//! crate's `oversampling()` flag. The *score* (what they play) lives in `crate::score`.

use fundsp::prelude32::*;

use super::oversampling;

/// Snare: high-passed noise crack + a short tone body + a clap layer (low-passed noise with slower
/// decay for that back-breaking demoscene crack — the clap fills the low-mids the crack misses).
pub(super) fn snare() -> Box<dyn AudioUnit> {
    Box::new(
        ((noise() >> highpass_hz(1200.0, 0.7)) * envelope(|t: f32| (-t * 26.0).exp())
            + sine_hz(190.0) * envelope(|t: f32| (-t * 24.0).exp()) * 0.5
            + (noise() >> highpass_hz(280.0, 0.6)) * envelope(|t: f32| (-t * 16.0).exp()) * 0.35)
            * 0.75,
    )
}

/// Hat: bright high-passed noise crack + a body layer (lower, slower) so it has a "tick" body
/// behind the sizzle — without it every hat is a wisp, not a percussion hit.
pub(super) fn hat() -> Box<dyn AudioUnit> {
    Box::new(
        ((noise() >> highpass_hz(7000.0, 0.7)) * envelope(|t: f32| (-t * 80.0).exp()) * 0.55
            + (noise() >> highpass_hz(3500.0, 0.5)) * envelope(|t: f32| (-t * 40.0).exp()) * 0.45)
            * 0.9,
    )
}

/// Stab: one chord note as a saw through a low-pass with a plucky attack (rendered per triad note
/// so the three can be panned wide).
pub(super) fn stab(freq: f32) -> Box<dyn AudioUnit> {
    Box::new(
        (saw_hz(freq) >> lowpass_hz(1600.0, 0.8) >> highpass_hz(180.0, 0.7))
            * envelope(|t: f32| {
                let a = 0.01;
                if t < a { t / a } else { (-(t - a) * 7.0).exp() }
            })
            * 0.3,
    )
}

/// Pad: one chord note an octave down through a soft low-pass, slow swell, high-passed off the
/// low-mids so it stops stacking into the same band as everything else (body/warmth, not honk).
pub(super) fn pad(freq: f32) -> Box<dyn AudioUnit> {
    Box::new(
        (saw_hz(freq * 0.5) >> lowpass_hz(900.0, 0.6) >> highpass_hz(150.0, 0.7))
            * envelope(|t: f32| (t * 2.0).min(1.0))
            * 0.22,
    )
}

/// Bass: a moving Reese — a sub sine + two ±8-cent-detuned saws (the phasing growl) through a
/// resonant low-pass that drops from ~1.4 kHz to ~900 Hz, with per-VOICE tanh drive so the grit
/// lives on the bass itself, not smeared across the whole bus.
pub(super) fn bass(freq: f32, vel: f32) -> Box<dyn AudioUnit> {
    let mk = move || {
        let saws = sine_hz(freq)
            + saw_hz(freq) * 0.6
            + saw_hz(freq * 1.008) * 0.5
            + saw_hz(freq * 0.992) * 0.5;
        let cut = envelope(|t: f32| 900.0 + 500.0 * (-t * 3.0).exp());
        let drive = 1.8 + 0.8 * vel; // harder notes growl harder
        ((saws | cut) >> lowpass_q(1.4) >> shape_fn(move |x| (x * drive).tanh()))
            * envelope(|t: f32| {
                let a = 0.005;
                if t < a { t / a } else { (-(t - a) * 4.0).exp() }
            })
            * 0.46
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// Wooz-bass: thick + dark in the low-mids, a slow GROWL that develops AFTER the hit, and a
/// slightly-detuned, woozy quality — the pitch never quite settles. How each trait is built:
///   • dark low-mid body — a sub sine for weight + two detuned saws, all through a RESONANT low-pass
///     parked low (~220 Hz, Q≈3.2) so it sits in the low-mids and never gets bright.
///   • growl-after-the-hit — the low-pass cutoff is WOBBLED by a ~5.5 Hz LFO whose depth ramps IN
///     over ~0.4 s, so the note lands clean and the growl only opens up as it sustains.
///   • woozy/unstable pitch — a ~4 Hz vibrato + a slower ~0.6 Hz drift on EACH oscillator (at
///     different rates/phases) on top of ±12-cent detuning, so the three voices beat against each
///     other and the pitch drifts. Best on HELD notes (it needs time to develop). A palette voice —
///     wire it into the score where a sustained woozy sub fits (`set woozbass=1` swaps it into the
///     bass note-lane; audition the voice alone with the `woozbass_demo` test).
pub(super) fn woozbass(freq: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    // independent vibrato + slow drift per oscillator → they never lock, so the pitch feels unstable.
    let f_sub = lfo(move |t: f32| {
        freq * (1.0 + 0.006 * (t * 4.3 * TAU).sin() + 0.004 * (t * 0.6 * TAU).sin())
    });
    let f_up = lfo(move |t: f32| freq * 1.007 * (1.0 + 0.006 * (t * 4.1 * TAU + 1.0).sin()));
    let f_dn = lfo(move |t: f32| freq * 0.993 * (1.0 + 0.005 * (t * 3.7 * TAU + 2.0).sin()));
    let oscs = (f_sub >> sine()) * 0.7 + (f_up >> saw()) * 0.45 + (f_dn >> saw()) * 0.45;
    // the developing growl: a resonant-LPF cutoff wobble whose depth eases in over ~0.4 s.
    let cut = lfo(move |t: f32| {
        let grow = (t / 0.4).min(1.0);
        220.0 + grow * 230.0 * ((t * 5.5 * TAU).sin() * 0.5 + 0.5)
    });
    Box::new(
        ((oscs | cut | constant(3.2)) >> lowpass())
            * envelope(|t: f32| {
                let a = 0.008;
                if t < a {
                    t / a // quick, clean attack...
                } else {
                    0.6 + 0.4 * (-(t - a) * 0.6).exp() // ...then a long sustain so the growl can bloom
                }
            })
            * 0.5,
    )
}

/// Lead: a 5-saw detuned stack with a per-note FILTER ENVELOPE — the cutoff sweeps down from ~4.9 kHz
/// to ~700 Hz so every note plucks/opens and settles instead of droning through a fixed cutoff (a
/// static cutoff on a saw is literally an organ). Softsign drive for brass bite; no sub-octave (that
/// read as an organ pipe).
pub(super) fn lead(freq: f32, vel: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    let mk = move || {
        // a gentle vibrato that SWELLS IN over the note — the lead leans into the note like a singer
        // instead of one static, ethereal tone. Each saw gets its own phase (lush, decorrelated).
        let vib = move |mult: f32, ph: f32| {
            lfo(move |t: f32| {
                let depth = 0.005 * (t * 1.4).min(1.0);
                freq * mult * (1.0 + depth * (t * 5.0 * TAU + ph).sin())
            })
        };
        let saws = ((vib(1.0, 0.0) >> saw())
            + (vib(1.007, 1.0) >> saw())
            + (vib(0.993, 2.0) >> saw())
            + (vib(1.014, 3.0) >> saw())
            + (vib(0.986, 4.0) >> saw()))
            * 0.18;
        // a higher floor + a slower sweep so the note stays PRESENT and bright (it SINGS) instead of
        // closing down to a thin/ethereal whisper; the sweep PEAK still tracks velocity.
        let top = 2400.0 + 2600.0 * vel;
        let cut = envelope(move |t: f32| 1550.0 + top * (-t * 3.4).exp());
        ((saws | cut) >> lowpass_q(0.8) >> shape(Softsign(0.4 + 0.4 * vel)))
            * envelope(|t: f32| {
                let a = 0.02;
                if t < a {
                    t / a
                } else {
                    0.55 + 0.45 * (-(t - a) * 0.85).exp() // high sustain floor → the note holds + SINGS
                }
            })
            * 0.8
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// Arp: short filtered pluck. Lower and quieter than the old bright square arp so it reads as
/// motion in the groove, not late-90s game melody.
pub(super) fn arp(freq: f32, vel: f32) -> Box<dyn AudioUnit> {
    let osc = saw_hz(freq) * 0.7 + square_hz(freq) * 0.15;
    let top = 2500.0 + 2500.0 * vel;
    let cut = envelope(move |t: f32| 600.0 + top * (-t * 22.0).exp());
    Box::new(
        ((osc | cut) >> lowpass_q(0.9) >> shape(Atan(0.5)))
            * envelope(|t: f32| {
                let a = 0.008;
                if t < a { t / a } else { (-(t - a) * 7.5).exp() }
            })
            * 0.24,
    )
}

/// Supersaw: 7 detuned saws + a sub-octave saw through a bright-ish filter, slow swell — the wide
/// "epic" chord wall for the drop/climax. Held a full bar per chord note (panned wide by chord_spread).
pub(super) fn supersaw(freq: f32) -> Box<dyn AudioUnit> {
    let mk = move || {
        let saws = (saw_hz(freq)
            + saw_hz(freq * 1.006)
            + saw_hz(freq * 0.994)
            + saw_hz(freq * 1.013)
            + saw_hz(freq * 0.987)
            + saw_hz(freq * 1.020)
            + saw_hz(freq * 0.980))
            * 0.13;
        let cut = envelope(|t: f32| 1300.0 + 3200.0 * (t * 1.0).min(1.0)); // filter swells open
        // HP off the sub, then DRIVE it (rawstyle screech grit) — a hard wall, not a polite pad.
        ((saws | cut) >> lowpass_q(0.7) >> highpass_hz(180.0, 0.7) >> shape(Tanh(1.8)))
            * envelope(|t: f32| (t * 3.0).min(1.0))
            * 0.42
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// Choir / ensemble pad: a wide bank of detuned saws + a sub-octave sine body through a soft filter
/// with a slow swell — lush grandeur layered UNDER the supersaw wall in the big sections (it carries
/// the warmth/size while the supersaw carries the bright edge). The new diffuse reverb makes it bloom.
pub(super) fn choir(freq: f32) -> Box<dyn AudioUnit> {
    let saws = (saw_hz(freq)
        + saw_hz(freq * 1.004)
        + saw_hz(freq * 0.996)
        + saw_hz(freq * 1.009)
        + saw_hz(freq * 0.991)
        + sine_hz(freq * 0.5) * 0.6)
        * 0.15;
    Box::new((saws >> lowpass_hz(2600.0, 0.7)) * envelope(|t: f32| (t * 1.0).min(1.0)) * 0.3)
}

/// Donk: a bright, plucky detuned-saw chord stab — the euphoric off-beat "donk" of happy-hardcore /
/// house / party music. Snappy filter pluck + an octave partial + a touch of drive so it cuts and
/// bounces on the up-beats.
pub(super) fn donk(freq: f32) -> Box<dyn AudioUnit> {
    let mk = move || {
        let saws =
            (saw_hz(freq) + saw_hz(freq * 1.01) + saw_hz(freq * 0.99) + saw_hz(freq * 2.0) * 0.4)
                * 0.2;
        let cut = envelope(|t: f32| 900.0 + 3600.0 * (-t * 16.0).exp());
        ((saws | cut) >> lowpass_q(1.0) >> shape(Tanh(1.4)))
            * envelope(|t: f32| {
                let a = 0.003;
                if t < a {
                    t / a
                } else {
                    (-(t - a) * 12.0).exp()
                }
            })
            * 0.4
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// House organ stab: the classic early-90s "M1 organ" rave/house chord stab — Haddaway "What Is Love",
/// Snap!, Cappella. A drawbar-organ tone (fundamental + octave + the nasal fifth + 2-octave partial,
/// like organ drawbars) with a hair of detuned saw for bite, a percussive pluck attack and a short
/// sustain through a bright resonant filter. Hollow + euphoric, but the drive + minor chords keep the
/// dark edge. Rendered per triad note so the chord can be panned wide.
pub(super) fn houseorg(freq: f32) -> Box<dyn AudioUnit> {
    let mk = move || {
        let organ = (sine_hz(freq)              // 16' fundamental
            + sine_hz(freq * 2.0) * 0.7         // 8'  octave
            + sine_hz(freq * 3.0) * 0.5         // 5⅓' fifth — the nasal organ honk
            + sine_hz(freq * 4.0) * 0.32        // 4'  two octaves up
            + saw_hz(freq * 1.005) * 0.28       // detuned saw pair = the "zaag" bite + width
            + saw_hz(freq * 0.995) * 0.28)
            * 0.17;
        let cut = envelope(|t: f32| 1200.0 + 3600.0 * (-t * 8.5).exp()); // bright pluck, settles fast
        ((organ | cut) >> lowpass_q(1.1) >> shape(Tanh(1.3)))
            * envelope(|t: f32| {
                let a = 0.004;
                if t < a {
                    t / a
                } else {
                    0.22 + 0.78 * (-(t - a) * 7.0).exp() // percussive attack → a short organ sustain
                }
            })
            * 0.44
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// CASIO / electric-piano: a tine-ish voice (sine carrier + a bell "ting" harmonic + a hair of saw
/// cheese) with a pluck-to-light-sustain envelope — the kitschy Ome-Henk keyboard comping.
pub(super) fn casio(freq: f32) -> Box<dyn AudioUnit> {
    let body = (sine_hz(freq)
        + sine_hz(freq * 2.01) * 0.45
        + sine_hz(freq * 4.02) * 0.18 // a slightly inharmonic bell "ting" (not a pure organ partial)
        + saw_hz(freq) * 0.07) // a hair of plastic cheese
        * 0.3;
    let cut = envelope(|t: f32| 800.0 + 3000.0 * (-t * 11.0).exp());
    Box::new(
        ((body | cut) >> lowpass_q(0.8) >> shape(Atan(0.4)))
            * envelope(|t: f32| {
                let a = 0.004;
                if t < a {
                    t / a
                } else {
                    0.12 + 0.88 * (-(t - a) * 6.5).exp() // a real pluck now, no organ sustain plateau
                }
            })
            * 0.5,
    )
}

// ============================================================================================
// PAD VARIANTS (padsw=2/3/4): alternate nocturnal pad CHARACTERS to audition (1 = the *_sw pair).
// Each = a wall-role (top) + a bed-role (body). Selected by the integer `padsw` knob in render.rs.
// ============================================================================================

/// Warm analog poly-pad — the WALL (Juno/OB-style bright-but-warm top).
/// The neon-air chord bed's upper layer, panned wide over the BED. Built to read UNMISTAKABLY as a
/// held analog pad with audible MOVEMENT (the old one was a faint wash):
///   • FIVE detuned saws, each on its own decorrelated drift LFO (no two rates match) so the chorus
///     never phase-locks — the chord keeps slowly beating/breathing for the whole held bar.
///   • TWO PWM `pulse()` oscillators whose duty slowly wobbles — the classic string-machine / OB
///     "animate" shimmer. This is a SECOND, independent motion layered on the saw drift, so the pad
///     has two different living movements instead of one static wash.
///   • the low-pass BLOOMS open 700→2900 Hz with a hair of resonance (lowpass_q 0.9) AND a slow
///     ~0.27 Hz cutoff tremolo riding on top, so the brightness gently undulates the whole note —
///     each chord swells in like a Juno volume pedal (attack curve powf 1.4), not a snap.
///   • Softsign(0.6) rounds the saw+pulse stack into warm analog thickness (warm, not buzzy); HP140
///     keeps it out of the bass; a whisper of octave sine adds glassy top. A touch hotter (*0.42) so
///     it actually registers, but still mix-safe under the lead.
pub(super) fn supersaw_sw2(freq: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    let mk = move || {
        // each saw drifts slowly around its detune offset on its own (rate, depth, phase) so the
        // ensemble chorus never locks — that slow beating is the "analog" life of the pad.
        let drift = move |mult: f32, rate: f32, depth: f32, ph: f32| {
            lfo(move |t: f32| mult * freq * (1.0 + depth * (t * rate * TAU + ph).sin()))
        };
        // string-machine PWM voice: a pulse whose DUTY slowly wobbles 0.32..0.68 → animated shimmer
        // that moves differently from the saw drift (a distinct second motion layer).
        let pwm = move |mult: f32, drate: f32, ddepth: f32, dph: f32, prate: f32, pph: f32| {
            ((lfo(move |t: f32| mult * freq * (1.0 + ddepth * (t * drate * TAU + dph).sin())))
                | (lfo(move |t: f32| 0.5 + 0.18 * (t * prate * TAU + pph).sin())))
                >> pulse()
        };
        let saws = (drift(1.000, 0.18, 0.0012, 0.0) >> saw())
            + (drift(1.007, 0.23, 0.0016, 1.3) >> saw())
            + (drift(0.993, 0.20, 0.0016, 2.6) >> saw())
            + (drift(1.011, 0.27, 0.0020, 3.9) >> saw())
            + (drift(0.989, 0.16, 0.0020, 5.2) >> saw());
        let pulses =
            pwm(1.004, 0.13, 0.0013, 0.7, 0.07, 0.0) + pwm(0.996, 0.15, 0.0013, 4.4, 0.05, 2.1);
        // saws carry the warm body, the PWM pulses add the moving string-machine shimmer on top.
        let osc = saws * 0.115 + pulses * 0.085 + sine_hz(freq * 2.0) * 0.02;
        // filter blooms open to a WARM peak (not a screech) with a slow tremolo so the brightness
        // keeps undulating for the whole held chord.
        let cut = envelope(|t: f32| {
            let bloom = 700.0 + 2200.0 * (1.0 - (-t / 0.9).exp());
            bloom * (1.0 + 0.10 * (t * 0.27 * TAU).sin())
        });
        ((osc | cut) >> lowpass_q(0.9) >> highpass_hz(140.0, 0.7) >> shape(Softsign(0.6)))
            * envelope(|t: f32| (t / 0.7).min(1.0).powf(1.4))
            * 0.42
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// Warm analog poly-pad — the BED (rounder/darker body UNDER the wall).
/// The warm bottom of the neon-air chord pad, panned wide opposite the WALL. Carries weight + warmth
/// so the pad has real body (the old bed was too subtle to register):
///   • FOUR detuned saws on slow decorrelated drift LFOs (~0.11–0.16 Hz) — a touch slower/deeper than
///     the wall so the bed breathes underneath rather than sparkling; the slow beating keeps it alive
///     for the whole held bar instead of sitting flat.
///   • a TRIANGLE at the fundamental for a soft, hollow Juno roundness, plus a sub-octave sine AND a
///     sub-SUB-octave sine for genuine low weight — this is the "size" you feel under the wall.
///   • a GENTLE low bloom 600→1700 Hz with a slow ~0.19 Hz cutoff wobble, under a long amp swell
///     (1.5 s, powf 1.3) → each chord blooms in like a deep breath, dark and warm, never bright.
///   • Softsign(0.35) for round analog thickness without honk; HP40 trims sub-rumble so the double
///     sub stays clean. Kept modest (*0.32) so it supports the wall and the lead, never masks them.
pub(super) fn choir_sw2(freq: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    let mk = move || {
        let drift = move |mult: f32, rate: f32, depth: f32, ph: f32| {
            lfo(move |t: f32| mult * freq * (1.0 + depth * (t * rate * TAU + ph).sin()))
        };
        let saws = (drift(1.000, 0.11, 0.0013, 0.0) >> saw())
            + (drift(1.005, 0.14, 0.0017, 1.9) >> saw())
            + (drift(0.995, 0.12, 0.0017, 3.3) >> saw())
            + (drift(1.009, 0.16, 0.0021, 4.7) >> saw());
        // saws for warmth, triangle for hollow Juno round, two sub-octave sines for real low weight.
        let body = saws * 0.13
            + (triangle_hz(freq) * 0.10)
            + (sine_hz(freq * 0.5) * 0.55)
            + (sine_hz(freq * 0.25) * 0.20);
        // gentle, dark bloom with a slow cutoff wobble → the bed breathes in under the wall.
        let cut = envelope(|t: f32| {
            let bloom = 600.0 + 1100.0 * (1.0 - (-t / 1.3).exp());
            bloom * (1.0 + 0.08 * (t * 0.19 * TAU + 1.1).sin())
        });
        ((body | cut) >> lowpass_q(0.7) >> highpass_hz(40.0, 0.6) >> shape(Softsign(0.35)))
            * envelope(|t: f32| (t / 1.5).min(1.0).powf(1.3))
            * 0.32
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// Neon-glass WALL — a vintage Solina / string-machine PWM pad (the bright, wide top of the bed).
/// Seven `pulse()` oscillators whose DUTY is swept by per-voice LFOs at ~0.43–0.67 Hz: that is the
/// classic PWM "string shimmer", a continuous animation of harmonic content the ear reads instantly
/// as a lush string pad (not a faint wash). Built to BE noticed:
///   • each voice also pitch-drifts on a decorrelated LFO so the ensemble never phase-locks — the slow
///     beating is the width + the "human" string warmth;
///   • a +octave saw adds glassy top air;
///   • the low-pass BLOOMS open 1100→3700 Hz over ~1.6 s so every held bar swells in like headlights;
///   • a slow ~0.31 Hz amplitude-tilt LFO gives the long undulating "neon air" pulse across the bar;
///   • HP170 keeps it clear of the bass, Softsign(0.6) glues the ensemble.
/// Held a full bar; panned wide against the warm BED beneath it. Mix-safe (*0.42).
pub(super) fn supersaw_sw3(freq: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    let mk = move || {
        // one PWM string voice: pitch drifts (drate/ddepth/dph) and duty sweeps (prate/pph), all
        // decorrelated so the ensemble keeps shimmering for the whole held chord.
        let pwm_saw = move |mult: f32, drate: f32, ddepth: f32, dph: f32, prate: f32, pph: f32| {
            let f = lfo(move |t: f32| mult * freq * (1.0 + ddepth * (t * drate * TAU + dph).sin()));
            let duty = lfo(move |t: f32| 0.5 + 0.42 * (t * prate * TAU + pph).sin());
            (f | duty) >> pulse()
        };
        let ens = ((pwm_saw(1.000, 0.17, 0.0010, 0.0, 0.45, 0.0)
            + pwm_saw(1.007, 0.21, 0.0012, 1.1, 0.53, 1.3)
            + pwm_saw(0.993, 0.19, 0.0012, 2.3, 0.49, 2.6)
            + pwm_saw(1.014, 0.25, 0.0014, 3.4, 0.61, 3.9)
            + pwm_saw(0.986, 0.23, 0.0014, 4.6, 0.57, 5.1)
            + pwm_saw(1.021, 0.27, 0.0016, 5.7, 0.67, 0.7)
            + pwm_saw(0.979, 0.15, 0.0016, 0.6, 0.43, 2.0))
            * 0.085)
            + (saw_hz(freq * 2.0) * 0.05); // glassy octave air on top
        // filter BLOOMS open slowly to a bright-but-warm glass peak — swell, not screech.
        let cut = envelope(|t: f32| 1100.0 + 2600.0 * (t / 1.6).min(1.0));
        // slow amplitude tilt = the undulating "neon air" pulse across the held bar.
        let tilt = lfo(|t: f32| 1.0 + 0.14 * (t * 0.31 * TAU).sin());
        ((ens | cut) >> lowpass_q(0.85) >> highpass_hz(170.0, 0.7) >> shape(Softsign(0.6)))
            * tilt
            * envelope(|t: f32| (t / 0.7).min(1.0))
            * 0.42
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// Neon-glass BED — the warm body UNDER the PWM string WALL so the pad never sounds thin.
/// A slow-drifting detuned-saw ensemble + one extra `pulse()` PWM string voice for matching shimmer +
/// a sub-octave sine for size, through a gently blooming low-pass. Recast so it breathes:
///   • five saws ride slow (~0.13–0.19 Hz) on decorrelated LFOs around their detune — the ensemble
///     keeps beating/shimmering (a real string section is never perfectly in tune);
///   • a single PWM `pulse()` voice (duty swept ~0.27 Hz) ties the bed to the wall's Solina character;
///   • the low-pass opens gently ~1.3→2.6 kHz over ~2 s under a long ~1.4 s amp swell, so each held
///     chord blooms in like a deep breath rather than snapping to full brightness;
///   • the sub-octave sine + Softsign(0.5) give round, glassy warmth and body without honk.
/// Kept under the wall (modest *0.32). Diffuse reverb downstream makes it bloom further.
pub(super) fn choir_sw3(freq: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    let mk = move || {
        let drift = move |mult: f32, rate: f32, depth: f32, ph: f32| {
            lfo(move |t: f32| mult * freq * (1.0 + depth * (t * rate * TAU + ph).sin()))
        };
        // one PWM string voice to share the wall's Solina shimmer down in the body.
        let pwm_body = {
            let f = lfo(move |t: f32| freq * (1.0 + 0.0010 * (t * 0.14 * TAU).sin()));
            let duty = lfo(|t: f32| 0.5 + 0.30 * (t * 0.27 * TAU).sin());
            (f | duty) >> pulse()
        };
        let body = ((drift(1.000, 0.13, 0.0010, 0.0) >> saw())
            + (drift(1.005, 0.17, 0.0012, 1.7) >> saw())
            + (drift(0.995, 0.15, 0.0012, 3.0) >> saw())
            + (drift(1.010, 0.19, 0.0014, 4.2) >> saw())
            + (drift(0.990, 0.16, 0.0014, 5.5) >> saw())
            + pwm_body * 0.5
            + sine_hz(freq * 0.5) * 0.7) // sub-octave body
            * 0.14;
        // soft filter blooms open slowly under a long swell → the bed breathes in.
        let cut = envelope(|t: f32| 1300.0 + 1300.0 * (t / 2.0).min(1.0));
        ((body | cut) >> lowpass_q(0.7) >> shape(Softsign(0.5)))
            * envelope(|t: f32| (t / 1.4).min(1.0))
            * 0.32
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// Pad WALL — dark wide cinematic wash (the bright-ish top, kept dark): 7 low detuned saws fused into
/// one thick driven sheet, with the headline MOVEMENT being a filter that audibly OPENS and CLOSES over
/// the held bar instead of a one-shot bloom that settles and vanishes. Recast for unmistakable presence:
///   • the resonant cutoff breathes on a slow ~0.55 Hz LFO — a full open→close cycle per bar — between a
///     dark floor (~700 Hz) and a warm peak (~2600 Hz), so you HEAR the chord inhale and exhale (this is
///     why it reads as a pad, not a wash). Deliberately never reaches bright/screech: brooding, not neon.
///   • a second, decorrelated ~0.31 Hz tremolo gently swells the amplitude so the sheet is never static.
///   • each saw drifts around its own detune on its own slow LFO (rate/depth/phase all different) so the
///     ensemble keeps beating like a real string machine and never locks for the whole held chord.
///   • Softsign(0.8) glues the saws into one warm, slightly-driven body; HP150 keeps it off the bass.
/// Blooms in over a long swell so each full-bar chord arrives like a deep breath. Reverb-hungry by design.
pub(super) fn supersaw_sw4(freq: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    let mk = move || {
        // each saw drifts slowly around its detune offset (rate, depth, phase all different) so the
        // wall keeps shimmering for the whole held chord — wide detune for a fat cinematic spread.
        let drift = move |mult: f32, rate: f32, depth: f32, ph: f32| {
            lfo(move |t: f32| mult * freq * (1.0 + depth * (t * rate * TAU + ph).sin()))
        };
        let saws = ((drift(1.000, 0.17, 0.0010, 0.0) >> saw())
            + (drift(1.007, 0.21, 0.0013, 1.1) >> saw())
            + (drift(0.993, 0.19, 0.0013, 2.3) >> saw())
            + (drift(1.014, 0.25, 0.0015, 3.4) >> saw())
            + (drift(0.986, 0.23, 0.0015, 4.6) >> saw())
            + (drift(1.022, 0.27, 0.0017, 5.7) >> saw())
            + (drift(0.978, 0.15, 0.0017, 0.6) >> saw()))
            * 0.12;
        // the MOVE: cutoff breathes a full open→close cycle (~0.55 Hz ≈ one per bar @124 BPM) between a
        // dark floor and a warm (not bright) peak — the pad audibly inhales and exhales. cos starts low.
        let cut = lfo(move |t: f32| {
            let breathe = 0.5 - 0.5 * (t * 0.55 * TAU).cos(); // 0→1→0 over the bar
            700.0 + 1900.0 * breathe
        });
        // slow decorrelated amplitude tremolo so the texture never sits still under the sweep.
        let trem = lfo(|t: f32| 0.85 + 0.15 * (t * 0.31 * TAU).sin());
        ((saws | cut) >> lowpass_q(0.8) >> highpass_hz(150.0, 0.7) >> shape(Softsign(0.8)))
            * trem
            * envelope(|t: f32| (t / 0.8).min(1.0)) // long bloom-in swell
            * 0.42
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// Pad BED — the deep sub/body layer under the dark cinematic wall. Where the wall carries the breathing
/// top, this carries the WEIGHT: a sub-octave + sub-sub sine give real low end, three slow-beating low
/// saws fill the warm midrange, and its own filter breathes too — slower and darker than the wall, so the
/// two layers phase against each other and the whole pad reads as a living, present thing (not a wash).
///   • sub-octave sine (freq*0.5) + sub-sub sine (freq*0.25) = the brooding cinematic floor / body.
///   • three saws ride slow (~0.13–0.17 Hz) with decorrelated detune LFOs so the ensemble keeps beating
///     (that slow phasing is the "human"/string-machine warmth — never perfectly in tune).
///   • the low-pass breathes on a slow ~0.41 Hz LFO between ~500 Hz and ~1500 Hz — it stays DARK and sits
///     UNDER the wall, and because its rate differs from the wall's the two never lock for the held bar.
///   • Softsign(0.5) rounds the body into glassy warmth without honk. Blooms in over a long amp swell.
/// Reverb makes it bloom further. Kept under the wall (modest gain) but deliberately PRESENT.
pub(super) fn choir_sw4(freq: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    let mk = move || {
        let drift = move |mult: f32, rate: f32, depth: f32, ph: f32| {
            lfo(move |t: f32| mult * freq * (1.0 + depth * (t * rate * TAU + ph).sin()))
        };
        let body = ((drift(1.000, 0.13, 0.0011, 0.0) >> saw())
            + (drift(1.006, 0.17, 0.0014, 1.7) >> saw())
            + (drift(0.994, 0.15, 0.0014, 3.0) >> saw())
            + sine_hz(freq * 0.5) * 0.85      // sub-octave = the warm body
            + sine_hz(freq * 0.25) * 0.5)     // sub-sub octave = the deep cinematic floor
            * 0.14;
        // dark filter breathes slower than the wall (~0.41 Hz) so the layers phase apart — stays UNDER.
        let cut = lfo(move |t: f32| {
            let breathe = 0.5 - 0.5 * (t * 0.41 * TAU).cos();
            500.0 + 1000.0 * breathe
        });
        ((body | cut) >> lowpass_q(0.7) >> shape(Softsign(0.5)))
            * envelope(|t: f32| (t / 1.4).min(1.0)) // slow deep-breath swell
            * 0.34
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

// ============================================================================================
// SYNTHWAVE PALETTE (`*_sw`): nocturnal Kavinsky/Midnight-flavoured alternates of the core voices,
// selected per-score by the `leadsw`/`basssw`/`padsw`/`drumsw` knobs (see render.rs). The default
// (knob 0) keeps the original bouncy/hard voices — e.g. the Camping show — unchanged.
// ============================================================================================

/// Lead: the signature nocturnal-synthwave HOOK. A sweet 3-saw core (unison + ±6 cents) for an
/// expressive, vocal detune instead of a 5-saw wall, a sine at the fundamental for rounded
/// breathy body (analog warmth, not an organ), and a quiet OCTAVE-UP saw shimmer so it floats
/// high and SINGS over the chord wall. A vibrato that SWELLS IN like a singer leaning into the
/// note (~5 Hz, slow, wistful). The per-note FILTER ENVELOPE opens bright then SETTLES to a warm,
/// present sustain (cinematic, not a static-cutoff organ, not a thin closing whisper) — the sweep
/// peak tracks velocity. A gentle ~190 Hz HIGH-PASS lifts it off the low-mids so it sits ON the
/// neon glass above the chords, and Atan drive (vel-scaled) gives round analog warmth + just
/// enough bite. Optional 2× oversample; modest gain, mix-safe.
pub(super) fn lead_sw(freq: f32, vel: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    let mk = move || {
        // a gentle vibrato that SWELLS IN over the note — the lead leans into the note like a singer
        // instead of one static tone. Slow (~5 Hz) + a touch deep so it reads wistful, not nervous.
        // Each oscillator gets its own phase (lush, decorrelated).
        let vib = move |mult: f32, ph: f32| {
            lfo(move |t: f32| {
                let depth = 0.006 * (t * 1.2).min(1.0);
                freq * mult * (1.0 + depth * (t * 5.0 * TAU + ph).sin())
            })
        };
        // a sweet 3-saw core (unison + ±6 cents) — expressive detune, not a 5-saw smear. A sine at the
        // fundamental adds rounded, breathy VOCAL body; a quiet octave-up saw adds shimmer so the
        // hook floats high and SINGS over the chord wall without going thin.
        let oscs =
            ((vib(1.0, 0.0) >> saw()) + (vib(1.006, 1.3) >> saw()) + (vib(0.994, 2.6) >> saw()))
                * 0.20
                + (vib(1.0, 0.7) >> sine()) * 0.16
                + (vib(2.0, 3.1) >> saw()) * 0.06;
        // open bright then SETTLE to a warm, present sustain (cinematic, not a static organ, not a
        // thin closing whisper). The sweep PEAK still tracks velocity.
        let top = 2200.0 + 2400.0 * vel;
        let cut = envelope(move |t: f32| 1600.0 + top * (-t * 3.0).exp());
        ((oscs | cut) >> lowpass_q(0.8) >> highpass_hz(190.0, 0.7) >> shape(Atan(0.5 + 0.4 * vel)))
            * envelope(|t: f32| {
                let a = 0.025;
                if t < a {
                    t / a // a soft glide-in so the note leans in, vocal, not a click
                } else {
                    0.58 + 0.42 * (-(t - a) * 0.8).exp() // high sustain floor → the note holds + SINGS
                }
            })
            * 0.8
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// Bass: the night-drive ENGINE — a moving Reese split into two bands so it goes DEEP without going
/// muddy. A clean SUB (sine an octave down + the fundamental) carries the weight; a separate
/// detuned-saw "growl" band (±9 cent, slowly creeping wider) is high-passed off the low end and
/// driven, so the grit sits ON TOP of an unmuddied sub. The growl's resonant cutoff drops
/// ~1.5 kHz → ~520 Hz AND breathes on a slow ~3 Hz wobble, so the Reese *moves* under the wheel
/// instead of droning. Per-VOICE tanh drive keeps the dirt on the bass, not the bus; a softer,
/// longer tail down to a low floor lets it sit under the sidechain pump and breathe.
pub(super) fn bass_sw(freq: f32, vel: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    let mk = move || {
        // clean sub spine: octave-down sine for the deep weight + the fundamental for body.
        let sub = sine_hz(freq * 0.5) + sine_hz(freq) * 0.55;
        // the Reese growl: a tight detune that slowly WIDENS so the beating opens up as the note holds.
        let det = move |sign: f32, ph: f32| {
            lfo(move |t: f32| {
                let spread = 0.009 + 0.004 * (t * 0.8).min(1.0); // detune creeps wider over the note
                freq * (1.0 + sign * spread) * (1.0 + 0.0015 * (t * 0.5 * TAU + ph).sin())
            })
        };
        let saws = (det(1.0, 0.0) >> saw()) * 0.55
            + (det(-1.0, 1.7) >> saw()) * 0.55
            + saw_hz(freq) * 0.35;
        // moving cutoff: a per-note drop that also gently breathes (~3 Hz) → the Reese never sits still.
        let cut = envelope(|t: f32| {
            let mv = 1.0 + 0.10 * (t * 3.0 * TAU).sin();
            (520.0 + 980.0 * (-t * 3.0).exp()) * mv
        });
        let drive = 2.0 + 1.0 * vel; // harder notes growl harder
        let growl = ((saws | cut) >> lowpass_q(1.5) >> highpass_hz(120.0, 0.6))
            >> shape_fn(move |x| (x * drive).tanh());
        // sub stays clean (just a gentle HP to clear DC); the growl band carries all the dirt.
        ((sub >> highpass_hz(28.0, 0.5)) * 0.7 + growl * 0.55)
            * envelope(|t: f32| {
                let a = 0.006;
                if t < a {
                    t / a
                } else {
                    // softer/longer decay to a low floor → breathes under the pump, doesn't pluck.
                    0.18 + 0.82 * (-(t - a) * 3.2).exp()
                }
            })
            * 0.46
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// Wooz-bass: a darker, deeper held-note GROWL for the brooding moments — neon under wet glass.
/// Thick + dark in the low-mids, a slow growl that blooms AFTER the hit, and a woozy pitch that
/// never quite settles. Recast for the nocturnal brief: a true SUB an octave down for real weight,
/// a SLOWER (~3.3 Hz) wobble so it broods rather than wibbles, and a resonance that EASES IN with
/// the wobble so the note lands clean and only grows teeth as it sustains. How each trait is built:
///   • deep dark body — a sub sine an OCTAVE down + the fundamental sine for weight, plus two
///     detuned saws, all through a RESONANT low-pass parked low (~210 Hz) so it stays in the
///     low-mids and never gets bright; a high-pass clears DC/rumble below ~26 Hz.
///   • growl-after-the-hit — the LPF cutoff is wobbled by a ~3.3 Hz LFO AND its resonance ramps from
///     soft (Q≈1.4) to fanged (Q≈3.4) over ~0.5 s, so the note lands clean and the growl blooms in.
///   • woozy/unstable pitch — a ~3.5 Hz vibrato + a slower ~0.55 Hz drift on EACH oscillator (at
///     different rates/phases) on top of ±9-cent detuning, so the voices beat against each other and
///     the pitch drifts. Best on HELD notes (it needs time to develop). A palette voice — wire it
///     into the score where a sustained woozy sub fits (`set woozbass=1` swaps it into the bass
///     note-lane; audition the voice alone with the `woozbass_demo` test).
pub(super) fn woozbass_sw(freq: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    // a true sub an octave down for deep, dark weight — gently woozy so even the spine drifts.
    let f_oct = lfo(move |t: f32| {
        freq * 0.5 * (1.0 + 0.004 * (t * 3.5 * TAU).sin() + 0.003 * (t * 0.55 * TAU).sin())
    });
    // independent vibrato + slow drift per oscillator → they never lock, so the pitch feels unstable.
    let f_sub = lfo(move |t: f32| {
        freq * (1.0 + 0.005 * (t * 3.5 * TAU).sin() + 0.003 * (t * 0.55 * TAU).sin())
    });
    let f_up = lfo(move |t: f32| freq * 1.009 * (1.0 + 0.005 * (t * 3.3 * TAU + 1.0).sin()));
    let f_dn = lfo(move |t: f32| freq * 0.991 * (1.0 + 0.004 * (t * 2.9 * TAU + 2.0).sin()));
    let oscs = (f_oct >> sine()) * 0.9
        + (f_sub >> sine()) * 0.45
        + (f_up >> saw()) * 0.4
        + (f_dn >> saw()) * 0.4;
    // the developing growl: a SLOW resonant-LPF cutoff wobble whose depth eases in over ~0.5 s.
    let cut = lfo(move |t: f32| {
        let grow = (t / 0.5).min(1.0);
        210.0 + grow * 220.0 * ((t * 3.3 * TAU).sin() * 0.5 + 0.5)
    });
    // resonance also ramps in (soft → fanged) so the note lands clean and grows teeth as it holds.
    let q = lfo(move |t: f32| 1.4 + 2.0 * (t / 0.5).min(1.0));
    Box::new(
        (((oscs | cut | q) >> lowpass()) >> highpass_hz(26.0, 0.5))
            * envelope(|t: f32| {
                let a = 0.008;
                if t < a {
                    t / a // quick, clean attack...
                } else {
                    0.6 + 0.4 * (-(t - a) * 0.6).exp() // ...then a long sustain so the growl can bloom
                }
            })
            * 0.5,
    )
}

/// Supersaw: 7 saws whose detuning slowly DRIFTS (each saw rides its own slow LFO) so the wall
/// shimmers and breathes like neon on wet glass instead of sitting at fixed, static offsets — the
/// wide chord wall for the drop/climax, recast nocturnal. How the night-drive character is built:
///   • blooming, not slamming — the filter eases from ~900 Hz up to a WARM ~3.3 kHz over ~1.6 s and
///     the amp swells in over ~0.6 s, so each chord opens like headlights coming on, not a hard hit.
///   • alive width — every saw's pitch wobbles on a slow (~0.15–0.27 Hz), decorrelated LFO around its
///     detune offset, so the seven voices keep beating against each other (glassy, evolving chorus).
///   • warm edge, not screech — Softsign drive (gentle saturation) replaces the rawstyle Tanh, HP at
///     150 Hz keeps the sub clean under the bass. Held a full bar per chord note (panned by spread).
pub(super) fn supersaw_sw(freq: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    let mk = move || {
        // each saw drifts slowly around its detune offset (rate, depth, phase all different) so the
        // chorus never locks — the wall keeps shimmering for the whole held chord.
        let drift = move |mult: f32, rate: f32, depth: f32, ph: f32| {
            lfo(move |t: f32| mult * freq * (1.0 + depth * (t * rate * TAU + ph).sin()))
        };
        let saws = ((drift(1.000, 0.17, 0.0009, 0.0) >> saw())
            + (drift(1.006, 0.21, 0.0011, 1.1) >> saw())
            + (drift(0.994, 0.19, 0.0011, 2.3) >> saw())
            + (drift(1.013, 0.25, 0.0013, 3.4) >> saw())
            + (drift(0.987, 0.23, 0.0013, 4.6) >> saw())
            + (drift(1.020, 0.27, 0.0015, 5.7) >> saw())
            + (drift(0.980, 0.15, 0.0015, 0.6) >> saw()))
            * 0.13;
        // filter BLOOMS open slowly to a warm peak (not a screech) — headlights, not a slam.
        let cut = envelope(|t: f32| 900.0 + 2400.0 * (t / 1.6).min(1.0));
        ((saws | cut) >> lowpass_q(0.7) >> highpass_hz(150.0, 0.7) >> shape(Softsign(0.7)))
            * envelope(|t: f32| (t / 0.6).min(1.0))
            * 0.4
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// Choir / ensemble pad: a slow-drifting detuned-saw ensemble + a sub-octave sine body through a soft,
/// slowly-BLOOMING low-pass — the warm pad bed layered UNDER the supersaw wall (it carries warmth/size
/// while the supersaw carries the bright edge). Recast nocturnal so it breathes instead of sitting flat:
///   • the saws ride slow (~0.13–0.19 Hz), decorrelated LFOs around their detune so the ensemble keeps
///     shimmering (a real choir is never perfectly in tune — that slow beating is the "human" warmth).
///   • the low-pass opens gently from ~1.4 kHz to ~2.4 kHz over ~2 s under a long amp swell, so each
///     held chord blooms in like a deep breath rather than snapping to full brightness.
///   • a sub-octave sine + a touch of Softsign give body and round, glassy warmth without honk.
/// The diffuse reverb makes it bloom further. Kept under the wall (modest gain).
pub(super) fn choir_sw(freq: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    let mk = move || {
        let drift = move |mult: f32, rate: f32, depth: f32, ph: f32| {
            lfo(move |t: f32| mult * freq * (1.0 + depth * (t * rate * TAU + ph).sin()))
        };
        let saws = ((drift(1.000, 0.13, 0.0010, 0.0) >> saw())
            + (drift(1.004, 0.17, 0.0012, 1.7) >> saw())
            + (drift(0.996, 0.15, 0.0012, 3.0) >> saw())
            + (drift(1.009, 0.19, 0.0014, 4.2) >> saw())
            + (drift(0.991, 0.16, 0.0014, 5.5) >> saw())
            + sine_hz(freq * 0.5) * 0.6)
            * 0.15;
        // soft filter blooms open slowly under a long swell → the pad breathes in.
        let cut = envelope(|t: f32| 1400.0 + 1000.0 * (t / 2.0).min(1.0));
        ((saws | cut) >> lowpass_q(0.7) >> shape(Softsign(0.4)))
            * envelope(|t: f32| (t / 1.4).min(1.0))
            * 0.3
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// Snare: a tight, dry GATED-analog snare for the night-drive — a short bright noise CRACK, a 180 Hz
/// tuned TONE body (the drum's pitch), a softer high-mid CLAP layer for fullness, and a hair of top
/// AIR so it snaps without washing. Drier + shorter than the old demoscene crack so it sits under the
/// lead instead of fighting it.
pub(super) fn snare_sw() -> Box<dyn AudioUnit> {
    Box::new(
        ((noise() >> highpass_hz(1500.0, 0.7)) * envelope(|t: f32| (-t * 34.0).exp())
            + sine_hz(180.0) * envelope(|t: f32| (-t * 30.0).exp()) * 0.45
            + (noise() >> highpass_hz(900.0, 0.6)) * envelope(|t: f32| (-t * 20.0).exp()) * 0.3
            + (noise() >> highpass_hz(8000.0, 0.6)) * envelope(|t: f32| (-t * 60.0).exp()) * 0.18)
            * 0.68,
    )
}

/// Hat: a crisp, metallic CLOSED-hat for the neon groove — three high-passed noise bands (a bright
/// sizzle, a metallic mid, and a low "tick" body) with a fast decay so it shimmers tight like neon on
/// wet glass instead of washing. Tighter + brighter than the old two-band wisp.
pub(super) fn hat_sw() -> Box<dyn AudioUnit> {
    Box::new(
        ((noise() >> highpass_hz(9000.0, 0.8)) * envelope(|t: f32| (-t * 95.0).exp()) * 0.5
            + (noise() >> highpass_hz(6000.0, 0.6)) * envelope(|t: f32| (-t * 70.0).exp()) * 0.32
            + (noise() >> highpass_hz(3800.0, 0.5)) * envelope(|t: f32| (-t * 55.0).exp()) * 0.22)
            * 0.85,
    )
}

// ============================================================================================
// LEAD VARIANTS (leadsw=2/3/4): alternate nocturnal lead CHARACTERS (1 = lead_sw). Selected by the
// integer `leadsw` knob in render.rs.
// ============================================================================================

/// Lead (breathy "sung"): a soft, intimate VOCAL topline — like a breathy sung melody. Built round
/// and throat-y from a SINE at the fundamental + a TRIANGLE doubling it (the two carry the body; no
/// saw wall), with only a HAIR of detuned saw for "air"/edge and a gentle BREATH-NOISE whisper
/// (band-passed noise that's loud on the consonant-like onset then fades back). An expressive
/// vibrato SWELLS IN over the note (~5.2 Hz, slow, a touch deep) so it leans into the phrase like a
/// singer. The filter stays MELLOW (low-Q lowpass parked ~1.4 kHz with a soft per-note open that
/// settles — never bright/buzzy) and a gentle ~150 Hz high-pass clears the low mud so it reads as a
/// throat, not a chest. A soft Atan rounds it; it sits a touch BACK in the mix (modest gain, no
/// oversample needed — there's almost no distortion to alias). Tender, wistful, almost human.
pub(super) fn lead_sw2(freq: f32, vel: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    // an expressive vibrato that SWELLS IN — the voice leans into the note. Slow (~5.2 Hz) + a touch
    // deep so it reads sung and wistful, not a nervous trill. Each oscillator gets its own phase.
    let vib = move |mult: f32, ph: f32| {
        lfo(move |t: f32| {
            let depth = 0.007 * (t * 0.9).min(1.0); // eases in over ~1.1 s like a held vocal note
            freq * mult * (1.0 + depth * (t * 5.2 * TAU + ph).sin())
        })
    };
    // round, throat-y BODY: a sine at the fundamental + a triangle doubling it (triangle's quiet odd
    // harmonics give a vowel-like body without the buzz of a saw). Only a HAIR of ±5-cent detuned
    // saw on top for "air"/edge so it isn't a pure flute — barely there.
    let body = (vib(1.0, 0.0) >> sine()) * 0.42
        + (vib(1.0, 1.7) >> triangle()) * 0.30
        + ((vib(1.005, 0.6) >> saw()) + (vib(0.995, 2.3) >> saw())) * 0.05;
    // a BREATH whisper: band-passed noise (a vocal-ish formant band) that's loud on the onset like a
    // consonant/aspiration, then fades back so it colours the attack and lingers faintly underneath.
    let breath =
        (noise() >> bandpass_hz(1800.0, 1.5)) * envelope(|t: f32| 0.10 + 0.22 * (-t * 9.0).exp());
    // MELLOW filter: parked low (~1.4 kHz) with a soft per-note open that SETTLES — never bright or
    // buzzy. Low Q so there's no resonant whistle; the sweep peak tracks velocity a little.
    let top = 700.0 + 900.0 * vel;
    let cut = envelope(move |t: f32| 1400.0 + top * (-t * 2.6).exp());
    let voice = ((body + breath) | cut) >> lowpass_q(0.6) >> highpass_hz(150.0, 0.6);
    Box::new(
        (voice >> shape(Atan(0.45)))
            * envelope(|t: f32| {
                let a = 0.045; // a soft breathy glide-in (longer than the saw lead) → no click, sung
                if t < a {
                    (t / a).powf(0.7)
                } else {
                    0.62 + 0.38 * (-(t - a) * 0.7).exp() // high sustain floor → it holds and SINGS
                }
            })
            * 0.62, // sits a touch BACK in the mix — intimate, not forward
    )
}

/// Lead: a bright, cutting, IN-YOUR-FACE BRASS-SAW hero (Carpenter-Brut / darkwave). A hard-detuned
/// 5-saw stack (WIDE ±1.1c / ±2.2c — a metal wall, not a sweet vocal shimmer) + an octave-up saw and
/// a hint of square for the brassy odd-harmonic edge that slices through a dense drop. Where the
/// nocturnal lead *leans in* like a singer, this one PUNCHES: a near-instant ~4 ms attack and a fast,
/// deep filter-PLUCK (cutoff slams ~1.7 kHz → up to ~9.4 kHz at full velocity, then collapses in
/// ~60 ms) so every note opens with a snappy bright bite and a hard plucky transient. Hefty velocity-
/// scaled Tanh drive (2.4–4.0) fed by a resonant low-pass gives the brassy grit + saturation that
/// reads as confident and edgy (Tanh's harder clip vs. a polite Atan/Softsign). A 240 Hz high-pass
/// keeps it lean and FORWARD so it sits on top of the mix. It still SINGS as a hook: the pluck settles
/// to a high 0.62 sustain floor (holds + rings, never a thin click), a slow detune LFO keeps living
/// motion under the held tone, and the played pitch carries clean across the whole stack. Optional
/// 2× oversample (the drive earns it); gain trimmed for the extra perceived loudness, mix-safe.
pub(super) fn lead_sw3(freq: f32, vel: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    let mk = move || {
        // each saw rides a slow detune LFO (own phase) — the WIDTH opens a touch over the note so the
        // brass wall keeps a living, aggressive beat instead of a static, frozen chord-of-one.
        let det = move |mult: f32, ph: f32| {
            lfo(move |t: f32| {
                let depth = 0.004 + 0.006 * (t * 0.9).min(1.0);
                freq * mult * (1.0 + depth * (t * 6.2 * TAU + ph).sin())
            })
        };
        // HARD 5-saw stack (±1.1c / ±2.2c — a thick metal wall) + octave-up saw shimmer + a touch of
        // square for the brassy odd-harmonic bite that cuts. Wider/harder than the sweet 3-saw lead.
        let oscs = ((det(1.0, 0.0) >> saw())
            + (det(1.011, 0.6) >> saw())
            + (det(0.989, 1.3) >> saw())
            + (det(1.022, 2.1) >> saw())
            + (det(0.978, 3.0) >> saw()))
            * 0.15
            + (det(2.0, 1.9) >> saw()) * 0.07
            + square_hz(freq) * 0.05;
        // a snappy bright filter-PLUCK that opens FAST and slams shut: cutoff peak tracks velocity hard
        // (~1.7 kHz → up to ~9.4 kHz), the steep -16 decay gives the hard transient snap (vs. the
        // nocturnal lead's slow cinematic settle). This is the aggressive Carpenter-Brut attack.
        let top = 5200.0 + 4200.0 * vel;
        let cut = envelope(move |t: f32| 1700.0 + top * (-t * 16.0).exp());
        // resonant low-pass (Q 1.3) feeding a hefty velocity-scaled Tanh = brassy grit + bite; the
        // 240 Hz high-pass keeps it lean and forward so it CUTS THROUGH the mix instead of muddying it.
        let drive = 2.4 + 1.6 * vel;
        ((oscs | cut) >> lowpass_q(1.3) >> highpass_hz(240.0, 0.7) >> shape(Tanh(drive)))
            * envelope(|t: f32| {
                let a = 0.004; // near-instant snap — the note hits HARD (not a vocal lean-in)
                if t < a {
                    t / a
                } else {
                    0.62 + 0.38 * (-(t - a) * 5.0).exp() // high sustain floor → holds + SINGS as a hook
                }
            })
            * 0.62 // trimmed for the drive's extra perceived loudness; mix-safe
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// Lead: a digital FM BELL-PLUCK — glassy, metallic 80s DX7 chime (think "E.PIANO"/"TUB BELLS").
/// Built from TRUE Chowning FM, not the saw wall: each operator is a CARRIER sine whose
/// instantaneous frequency is bent at audio rate by a MODULATOR sine summed straight into its
/// frequency INPUT (fundsp's `sine` integrates that to phase per-sample → real phase modulation,
/// NOT a control-rate `lfo` approximation). The LOW pair uses an inharmonic 1:3.5 ratio for the
/// clangorous struck-metal attack; its modulation index (peak deviation, in Hz) DECAYS fast — the
/// signature DX7 bell evolution where the metallic clang melts into a pure ringing sine. A HIGH
/// octave-up, near-inharmonic 1:1.41 pair adds crystalline glint, and a clean fundamental sine
/// seals a glassy ring body. The amplitude is a near-instant PLUCK strike with a long exponential
/// ring tail down to a small sustain floor (0.08) so the hook RINGS and SINGS between notes; a slow
/// vibrato (~5 Hz, swells in) leans into that tail like a singer so it reads as a melody, not a
/// bleep. A ~200 Hz HIGH-PASS lifts the chime off the low-mids onto the neon glass and a vel-scaled
/// Atan rounds the edges. Optional 2× oversample; modest gain, mix-safe.
pub(super) fn lead_sw4(freq: f32, vel: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    let mk = move || {
        // one FM operator pair: a CARRIER sine whose freq input is bent by a MODULATOR sine summed
        // in at audio rate (true Chowning FM — integrated to phase inside `sine`). The modulation
        // index (peak deviation in Hz = idx * mod_freq) DECAYS fast → metallic clang → pure ring.
        // `ratio` sets harmonicity (inharmonic ratios = bell, not buzz). `vph` decorrelates vibrato.
        let op = move |cf: f32, ratio: f32, idx0: f32, dec: f32, vph: f32| {
            let mf = cf * ratio;
            // modulator amplitude (the index, smooth/sub-20 Hz so a control-rate envelope is fine);
            // scaled by mf to express it as a Hz deviation, decaying over the note.
            let modu = sine_hz(mf) * envelope(move |t: f32| mf * idx0 * (-t * dec).exp());
            // a faint vibrato that SWELLS IN leans the carrier into the ring tail (singer, not bleep).
            let carrier = lfo(move |t: f32| {
                let v = 0.004 * (t * 0.6).min(1.0) * (t * 5.0 * TAU + vph).sin();
                cf * (1.0 + v)
            });
            (modu + carrier) >> sine()
        };
        // LOW pair: fundamental carrier + inharmonic 1:3.5 modulator → the struck-bell clang.
        let lo = op(freq, 3.5, 1.5 + 1.3 * vel, 8.5, 0.0);
        // HIGH pair: an octave up, near-inharmonic 1:1.41 → crystalline glint over the top.
        let hi = op(freq * 2.0, 1.41, 0.7 + 0.5 * vel, 13.0, 1.7);
        // a clean sine at the fundamental seals the ring tail with a pure, glassy body.
        let body = sine_hz(freq) * 0.10;
        let voice = lo * 0.5 + hi * 0.2 + body;
        // PLUCK amplitude: a near-instant strike, then a long exponential ring tail to a small
        // sustain floor so the hook HOLDS + SINGS between notes instead of dying to nothing.
        let amp = envelope(|t: f32| {
            let a = 0.003;
            if t < a {
                t / a
            } else {
                0.08 + 0.92 * (-(t - a) * 2.6).exp()
            }
        });
        // HP lifts the chime off the low-mids onto the neon glass; gentle vel-scaled Atan rounds it.
        (voice * amp >> highpass_hz(200.0, 0.7) >> shape(Atan(0.35 + 0.3 * vel))) * 0.8
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

// ============================================================================================
// BASS VARIANTS (basssw=2/3/4): alternate nocturnal bass CHARACTERS, each a moving-bass + held
// (woozbass) pair (1 = bass_sw/woozbass_sw). Selected by the integer `basssw` knob in render.rs.
// ============================================================================================

/// Bass — clean deep pulsing sub (the Kavinsky "Nightcall" engine). A round, sine-dominant
/// sub with only a whisper of grit, built to PUMP under the sidechain. How each trait is built:
///   • deep & clean — an OCTAVE-DOWN sine carries the weight + the fundamental sine for body;
///     this spine gets only a gentle HP at ~26 Hz (clears DC/rumble) and NO drive, so the
///     low end stays pure and never muddy.
///   • whisper of definition — a faint saw+triangle layer gives the note an edge so it reads
///     as pitched, not a pure test tone. It is QUARANTINED above ~120 Hz with a high-pass and
///     fed only a gentle tanh, so its grit can't smear down into the sub band.
///   • dark pulse — the grit-layer cutoff is parked LOW (~240 Hz, a tight 520 Hz click on the
///     attack) with a small ~4 Hz breathe, so it punches and pulses but never opens bright.
///   • sidechain-friendly — the amp decays to a HIGH floor (~0.22) over a soft tail instead of
///     plucking, so the duck carves a clean pump and the sub swells back between kicks.
/// Velocity nudges the (still gentle) drive; ends in the oversample wrapper since it has drive.
pub(super) fn bass_sw2(freq: f32, vel: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    let mk = move || {
        // clean sub spine: octave-down sine for the deep weight + the fundamental for body.
        let sub = sine_hz(freq * 0.5) * 1.0 + sine_hz(freq) * 0.32;
        // a whisper of saw+triangle for note definition — kept tiny so it never dominates.
        let def = saw_hz(freq) * 0.16 + triangle_hz(freq) * 0.12;
        // dark, breathing cutoff: a tight click on the attack settling LOW, a small ~4 Hz pulse.
        let cut = envelope(|t: f32| {
            let breathe = 1.0 + 0.06 * (t * 4.0 * TAU).sin();
            (240.0 + 520.0 * (-t * 6.0).exp()) * breathe
        });
        let drive = 1.15 + 0.45 * vel; // harder notes get a touch more edge — still gentle
        // the definition layer is HP'd above 120 Hz so its (light) grit can't smear the sub.
        let grit = ((def | cut) >> lowpass_q(0.9) >> highpass_hz(120.0, 0.6))
            >> shape_fn(move |x| (x * drive).tanh());
        // sub stays clean (just a gentle HP to clear DC); the grit rides quietly on top.
        ((sub >> highpass_hz(26.0, 0.5)) * 0.9 + grit * 0.5)
            * envelope(|t: f32| {
                let a = 0.004;
                if t < a {
                    t / a
                } else {
                    // soft tail to a HIGH floor → breathes under the pump, doesn't pluck.
                    0.22 + 0.78 * (-(t - a) * 2.6).exp()
                }
            })
            * 0.46
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// Wooz-bass: the HELD Kavinsky sub — a long, near-pure sustained sub pillow that sits under the
/// chords. Where the moving bass pulses, this one just holds, deep and almost clean. How each
/// trait is built:
///   • deep pillow body — an OCTAVE-DOWN sine carries the weight + the fundamental sine for body,
///     plus a faint triangle for a hair of edge; everything runs through a low-pass parked VERY
///     low (~180–270 Hz, gentle Q 0.8) so it stays a dark cushion and never gets bright.
///   • barely-there motion — instead of a wobble/growl this uses only a tiny (±0.15–0.18 %) slow
///     drift per sine + a ~2 Hz cutoff sway, so it shimmers faintly and feels alive without ever
///     becoming a Reese. The pitch stays essentially stable (it's a pillow, not a creature).
///   • clean & deep-not-muddy — a high-pass at ~24 Hz clears DC/rumble; no drive at all, so the
///     sub band is pure. The cushion opens a touch over ~0.6 s so it settles in under the chords.
///   • sidechain-friendly — a long sustain to a HIGH floor (0.7) so it breathes under the pump and
///     fills the gaps between kicks. Best on HELD notes. (`set woozbass=1` swaps it into the bass
///     note-lane; audition it alone with the `woozbass_demo` test.)
pub(super) fn woozbass_sw2(freq: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    // octave-down sub spine with a barely-there slow drift — deep weight, gently alive.
    let f_oct = lfo(move |t: f32| freq * 0.5 * (1.0 + 0.0015 * (t * 0.4 * TAU).sin()));
    // fundamental with its own slow drift (different rate/phase) so it shimmers, never wobbles.
    let f_fun = lfo(move |t: f32| freq * (1.0 + 0.0018 * (t * 0.33 * TAU + 1.0).sin()));
    let oscs = (f_oct >> sine()) * 1.0 + (f_fun >> sine()) * 0.4 + triangle_hz(freq) * 0.08;
    // a dark cushion that opens a hair over ~0.6 s, with a slow ~2 Hz sway (no growl).
    let cut = lfo(move |t: f32| {
        let open = (t / 0.6).min(1.0);
        180.0 + open * 90.0 + 35.0 * (t * 2.0 * TAU).sin()
    });
    Box::new(
        (((oscs | cut | constant(0.8)) >> lowpass()) >> highpass_hz(24.0, 0.5))
            * envelope(|t: f32| {
                let a = 0.012;
                if t < a {
                    t / a // gentle, clean attack...
                } else {
                    0.7 + 0.3 * (-(t - a) * 0.5).exp() // ...then a long, high-floor sustain
                }
            })
            * 0.5,
    )
}

/// Dirty Reese growl — the moving low-end ENGINE (Carpenter Brut darkwave menace). A clean SUB
/// spine (octave-down sine + fundamental) carries the deep weight and is kept entirely apart from
/// the dirt. On top, a wide, hard-beating detuned-saw Reese (with a square edge for extra teeth)
/// runs through a RESONANT moving low-pass and per-VOICE Tanh drive, so it snarls and cuts. How the
/// character is built — and how it stays DEEP but never muddy / sidechain-friendly:
///   • clean tight sub — `sine(freq*0.5)+sine(freq)*0.5`, only a DC-clearing HP30; the bottom octave
///     is a pure sine and never sees the drive, so the low stays tight.
///   • quarantined grit — the Reese band is HIGH-PASSED at 120 Hz *before* the Tanh, so all the
///     saturation harmonics live ABOVE the sub and can't smear it.
///   • the GROWL moves — a 3-input resonant lowpass whose cutoff drops ~2.3 kHz → 560 Hz per note
///     AND breathes on a ~4.5 Hz wobble, while Q climbs from 2.4 into the fangs and the detune slowly
///     WIDENS, so the Reese snarls and opens up instead of droning.
///   • sidechain-friendly — no pluck: the amp falls to a high sustain floor (0.2) with a slow time
///     constant, giving a soft, long tail with no transient of its own that breathes under the pump.
/// Ends in the oversample wrapper because it carries drive.
pub(super) fn bass_sw3(freq: f32, vel: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    let mk = move || {
        // clean sub spine: octave-down sine for deep weight + the fundamental for body.
        let sub = sine_hz(freq * 0.5) + sine_hz(freq) * 0.5;
        // the Reese growl: a wide detune that slowly WIDENS so the beating opens as the note holds;
        // each oscillator also gets a tiny slow drift so the voices never lock into a static phase.
        let det = move |sign: f32, ph: f32| {
            lfo(move |t: f32| {
                let spread = 0.014 + 0.006 * (t * 1.0).min(1.0); // detune creeps wider over the note
                freq * (1.0 + sign * spread) * (1.0 + 0.002 * (t * 0.6 * TAU + ph).sin())
            })
        };
        // two wide saws beating hard + a square for extra teeth + the fundamental saw for body.
        let saws = (det(1.0, 0.0) >> saw()) * 0.6
            + (det(-1.0, 2.1) >> saw()) * 0.6
            + (det(1.0, 4.0) >> square()) * 0.25
            + saw_hz(freq) * 0.4;
        // moving cutoff: a per-note drop (2.3 kHz → 560 Hz) that also breathes on a ~4.5 Hz wobble.
        let cut = envelope(|t: f32| {
            let wob = 1.0 + 0.22 * (t * 4.5 * TAU).sin();
            (560.0 + 1700.0 * (-t * 4.0).exp()) * wob
        });
        // resonance climbs into the fangs over the first ~0.5 s → the filter grows teeth as it moves.
        let q = envelope(|t: f32| 2.4 + 1.2 * (t * 2.0).min(1.0));
        let drive = 2.6 + 1.6 * vel; // harder notes growl harder
        // GRIT QUARANTINE: HP120 the growl band *before* the Tanh so the saturation can't smear the sub.
        let growl = ((saws | cut | q) >> lowpass())
            >> highpass_hz(120.0, 0.7)
            >> shape(Tanh(1.0))
            >> shape_fn(move |x| (x * drive).tanh());
        // sub stays clean (just a DC-clearing HP); the growl band carries all the dirt.
        ((sub >> highpass_hz(30.0, 0.5)) * 0.62 + growl * 0.6)
            * envelope(|t: f32| {
                let a = 0.005;
                if t < a {
                    t / a
                } else {
                    // soft, long tail to a high floor → no pluck transient; breathes under the pump.
                    0.2 + 0.8 * (-(t - a) * 3.4).exp()
                }
            })
            * 0.46
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// Dirty Reese growl — the HELD/sustained variant: a long, evolving menacing growl that OPENS UP as
/// it sustains. Same darkwave engine as the moving voice, but recast for long notes — instead of a
/// per-note drop-and-pluck, the filter (cutoff AND resonance) slowly BLOOMS open over ~1.2 s, so the
/// note lands relatively dark and grows fangs the longer it is held. How each trait is built — and
/// how it stays DEEP but never muddy / sidechain-friendly:
///   • deep tight sub — a separate clean spine (octave-down sine + fundamental), only an HP26 to clear
///     DC; it never touches the drive, so the bottom octave stays a pure, tight sine.
///   • quarantined grit — the wide detuned-saw + square Reese band is HIGH-PASSED at 120 Hz *before*
///     the Tanh, so the menace lives above the sub and can never smear the low.
///   • opens up as it sustains — cutoff eases 300 Hz → ~1 kHz over ~1.2 s with a slow ~0.7 Hz sweep
///     and a faster ~3.6 Hz wobble, while Q ramps 1.6 → 4.0; the growl blooms and broods.
///   • woozy menace — each oscillator rides its own vibrato (different rates/phases) on top of
///     ±1.3-cent detune so the voices beat and the pitch never quite settles.
///   • sidechain-friendly — a very long, high sustain (floor 0.62, slow decay) with no transient of
///     its own, so it sits and breathes under the pump. A palette voice for the brooding held moments.
pub(super) fn woozbass_sw3(freq: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    // a true sub an octave down for deep weight — only barely woozy so the spine stays solid.
    let f_oct = lfo(move |t: f32| freq * 0.5 * (1.0 + 0.003 * (t * 3.0 * TAU).sin()));
    // independent vibrato per oscillator → they never lock, so the growl shimmers and feels unstable.
    let det = move |mult: f32, rate: f32, ph: f32| {
        lfo(move |t: f32| freq * mult * (1.0 + 0.005 * (t * rate * TAU + ph).sin()))
    };
    // the Reese: octave-down body + a wide detuned-saw pair + a square edge for darkwave teeth.
    let oscs = (f_oct >> sine()) * 0.85
        + (det(1.0, 3.5, 0.0) >> saw()) * 0.5
        + (det(1.013, 3.1, 1.0) >> saw()) * 0.5
        + (det(0.987, 2.7, 2.0) >> saw()) * 0.5
        + (det(1.013, 2.3, 3.0) >> square()) * 0.22;
    // BLOOM: cutoff opens 300 → ~1 kHz over ~1.2 s with a slow sweep + a faster wobble → grows fangs.
    let cut = lfo(move |t: f32| {
        let grow = (t / 1.2).min(1.0);
        300.0
            + grow * (700.0 + 600.0 * ((t * 0.7 * TAU).sin() * 0.5 + 0.5))
            + grow * 180.0 * (t * 3.6 * TAU).sin()
    });
    // resonance ramps in (soft → fanged) over ~0.8 s so the menace builds as the note sustains.
    let q = lfo(move |t: f32| 1.6 + 2.4 * (t / 0.8).min(1.0));
    // GRIT QUARANTINE: HP120 the growl band *before* the Tanh so the dirt can't smear the sub.
    let growl = (((oscs | cut | q) >> lowpass()) >> highpass_hz(120.0, 0.7)) >> shape(Tanh(2.2));
    // a separate CLEAN sub spine for tight deep weight — never sees the drive (just a DC-clearing HP).
    let subspine = (sine_hz(freq * 0.5) + sine_hz(freq) * 0.4) >> highpass_hz(26.0, 0.5);
    Box::new(
        (subspine * 0.55 + growl * 0.6)
            * envelope(|t: f32| {
                let a = 0.01;
                if t < a {
                    t / a // quick, clean attack...
                } else {
                    // ...then a very long, high sustain so the growl can bloom + breathe under the pump.
                    0.62 + 0.38 * (-(t - a) * 0.5).exp()
                }
            })
            * 0.5,
    )
}

/// Acid bass: a 303-style squelch — saw+pulse through a RESONANT low-pass whose cutoff is swept by a
/// fast per-note env AND a slow ~3 Hz wobble (the "talking" filter), with a soft slide into pitch and
/// tanh drive for that rubbery acid bark. The sub spine (octave-down sine) is kept CLEAN and parallel
/// so all the resonant grit lives only in a band high-passed above 120 Hz — deep but never muddy.
/// Sidechain-friendly: a soft, long amp tail so the low end breathes back in under the pump.
pub(super) fn bass_sw4(freq: f32, vel: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    let mk = move || {
        // SLIDE: the pitch glides up into the target over ~40 ms → the portamento "303 slide" feel.
        let glide = lfo(move |t: f32| {
            let p = (t / 0.04).min(1.0);
            freq * (0.945 + 0.055 * p)
        });
        // 303 osc: a saw with a hair of pulse for the hollow squelch; both ride the same slide.
        let glide2 = glide.clone();
        let osc = (glide >> saw()) * 0.8 + ((glide2 | constant(0.32)) >> pulse()) * 0.35;

        // clean sub spine, octave down — carries the weight, never touched by the acid filter.
        let sub = sine_hz(freq * 0.5) * 0.9;

        // the TALKING filter: a fast per-note sweep (high→low) that also breathes on a ~3 Hz wobble;
        // accent (vel) opens it brighter & adds more resonance — that's the squelch "wow".
        let top = 2600.0 + 2600.0 * vel;
        let cut = lfo(move |t: f32| {
            let env = 380.0 + top * (-t * 9.0).exp(); // the squelch sweep
            let wob = 1.0 + 0.18 * (t * 3.0 * TAU).sin(); // slow breathing on top
            (env * wob).clamp(90.0, 7000.0)
        });
        // resonance blooms in over the first ~80 ms (lands clean, then squelches); accent adds fang.
        let q = lfo(move |t: f32| (2.6 + 3.4 * vel) * (0.55 + 0.45 * (t / 0.08).min(1.0)));
        let drive = 2.2 + 1.4 * vel; // harder notes bark harder

        // all the grit stays ABOVE 120 Hz so it can never smear the clean sub underneath.
        let squelch = ((osc | cut | q) >> lowpass())
            >> highpass_hz(120.0, 0.7)
            >> shape_fn(move |x: f32| (x * drive).tanh());

        ((sub >> highpass_hz(28.0, 0.5)) * 0.7 + squelch * 0.6)
            * envelope(|t: f32| {
                let a = 0.004;
                if t < a {
                    t / a
                } else {
                    // a softer, longer tail so the acid breathes under the sidechain pump.
                    0.16 + 0.84 * (-(t - a) * 4.5).exp()
                }
            })
            * 0.46
    };
    if oversampling() {
        Box::new(oversample(mk()))
    } else {
        Box::new(mk())
    }
}

/// Held acid bass: keeps the 303 squelch ALIVE over a long note — instead of a one-shot pluck the
/// filter slowly CYCLES (a ~0.45 Hz LFO sweeping cutoff + a faster ~3.3 Hz wobble) and the resonance
/// blooms in, so a sustained note keeps "talking" the whole way through. Clean octave-down sub spine
/// runs in parallel for the weight; all the resonant grit is quarantined above 120 Hz so the sub
/// stays deep but never muddy. Long sustain → breathes under the sidechain pump. Best on HELD notes.
pub(super) fn woozbass_sw4(freq: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    // a gentle slide into the note; the same glide feeds both oscillators (real portamento).
    let glide = lfo(move |t: f32| {
        let p = (t / 0.05).min(1.0);
        freq * (0.94 + 0.06 * p)
    });
    let glide2 = glide.clone();
    let osc = (glide >> saw()) * 0.8 + ((glide2 | constant(0.3)) >> pulse()) * 0.35;

    // clean sub spine, octave down — the deep weight, untouched by the acid filter.
    let sub = sine_hz(freq * 0.5) * 0.95;

    // a slow filter CYCLE (the held note keeps opening & closing) + a faster wobble on top.
    let cut = lfo(move |t: f32| {
        let cycle = (t * 0.45 * TAU).sin() * 0.5 + 0.5; // 0..1 slow open/close
        let wob = (t * 3.3 * TAU).sin() * 0.5 + 0.5;
        (300.0 + 1700.0 * cycle + 320.0 * wob).clamp(90.0, 5000.0)
    });
    // resonance ramps soft→fanged over ~0.5 s so it lands clean then grows teeth as it holds.
    let q = lfo(move |t: f32| 3.2 + 3.0 * (t / 0.5).min(1.0));

    // grit lives only above 120 Hz; the clean sub below stays out of the dirt.
    let squelch = ((osc | cut | q) >> lowpass())
        >> highpass_hz(120.0, 0.7)
        >> shape_fn(|x: f32| (x * 2.4).tanh());

    Box::new(
        ((sub >> highpass_hz(26.0, 0.5)) * 0.75 + squelch * 0.6)
            * envelope(|t: f32| {
                let a = 0.008;
                if t < a {
                    t / a // quick, clean attack...
                } else {
                    0.6 + 0.4 * (-(t - a) * 0.6).exp() // ...then a long sustain so the squelch can cycle
                }
            })
            * 0.5,
    )
}
