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

/// Snare: high-passed noise crack + a short tone body + a clap layer (low-passed noise with slower
/// decay for that back-breaking demoscene crack — the clap fills the low-mids the crack misses).
fn snare() -> Box<dyn AudioUnit> {
    Box::new(
        ((noise() >> highpass_hz(1200.0, 0.7)) * envelope(|t: f32| (-t * 26.0).exp())
            + sine_hz(190.0) * envelope(|t: f32| (-t * 24.0).exp()) * 0.5
            + (noise() >> highpass_hz(280.0, 0.6)) * envelope(|t: f32| (-t * 16.0).exp()) * 0.35)
            * 0.75,
    )
}

/// Hat: bright high-passed noise crack + a body layer (lower, slower) so it has a "tick" body
/// behind the sizzle — without it every hat is a wisp, not a percussion hit.
fn hat() -> Box<dyn AudioUnit> {
    Box::new(
        ((noise() >> highpass_hz(7000.0, 0.7)) * envelope(|t: f32| (-t * 80.0).exp()) * 0.55
            + (noise() >> highpass_hz(3500.0, 0.5)) * envelope(|t: f32| (-t * 40.0).exp()) * 0.45)
            * 0.9,
    )
}

/// Stab: one chord note as a saw through a low-pass with a plucky attack (rendered per triad note
/// so the three can be panned wide).
fn stab(freq: f32) -> Box<dyn AudioUnit> {
    Box::new(
        (saw_hz(freq) >> lowpass_hz(1600.0, 0.8) >> highpass_hz(180.0, 0.7))
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

/// Pad: one chord note an octave down through a soft low-pass, slow swell, high-passed off the
/// low-mids so it stops stacking into the same band as everything else (body/warmth, not honk).
fn pad(freq: f32) -> Box<dyn AudioUnit> {
    Box::new(
        (saw_hz(freq * 0.5) >> lowpass_hz(900.0, 0.6) >> highpass_hz(150.0, 0.7))
            * envelope(|t: f32| (t * 2.0).min(1.0))
            * 0.22,
    )
}

/// Bass: a moving Reese — a sub sine + two ±8-cent-detuned saws (the phasing growl) through a
/// resonant low-pass that drops from ~1.4 kHz to ~900 Hz, with per-VOICE tanh drive so the grit
/// lives on the bass itself, not smeared across the whole bus.
fn bass(freq: f32, vel: f32) -> Box<dyn AudioUnit> {
    let saws = sine_hz(freq)
        + saw_hz(freq) * 0.6
        + saw_hz(freq * 1.008) * 0.5
        + saw_hz(freq * 0.992) * 0.5;
    let cut = envelope(|t: f32| 900.0 + 500.0 * (-t * 3.0).exp());
    let drive = 1.8 + 0.8 * vel; // harder notes growl harder
    Box::new(
        ((saws | cut) >> lowpass_q(1.4) >> shape_fn(move |x| (x * drive).tanh()))
            * envelope(|t: f32| {
                let a = 0.005;
                if t < a {
                    t / a
                } else {
                    (-(t - a) * 4.0).exp()
                }
            })
            * 0.46,
    )
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
fn woozbass(freq: f32) -> Box<dyn AudioUnit> {
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
fn lead(freq: f32, vel: f32) -> Box<dyn AudioUnit> {
    use std::f32::consts::TAU;
    // a gentle vibrato that SWELLS IN over the note — the lead leans into the note like a singer
    // instead of sitting as one static, ethereal tone. Each saw gets its own phase (lush, decorrelated).
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
    Box::new(
        ((saws | cut) >> lowpass_q(0.8) >> shape(Softsign(0.4 + 0.4 * vel)))
            * envelope(|t: f32| {
                let a = 0.02;
                if t < a {
                    t / a
                } else {
                    0.55 + 0.45 * (-(t - a) * 0.85).exp() // high sustain floor → the note holds + SINGS
                }
            })
            * 0.8,
    )
}

/// Arp: short filtered pluck. Lower and quieter than the old bright square arp so it reads as
/// motion in the groove, not late-90s game melody.
fn arp(freq: f32, vel: f32) -> Box<dyn AudioUnit> {
    let osc = saw_hz(freq) * 0.7 + square_hz(freq) * 0.15;
    let top = 2500.0 + 2500.0 * vel;
    let cut = envelope(move |t: f32| 600.0 + top * (-t * 22.0).exp());
    Box::new(
        ((osc | cut) >> lowpass_q(0.9) >> shape(Atan(0.5)))
            * envelope(|t: f32| {
                let a = 0.008;
                if t < a {
                    t / a
                } else {
                    (-(t - a) * 7.5).exp()
                }
            })
            * 0.24,
    )
}

/// Supersaw: 7 detuned saws + a sub-octave saw through a bright-ish filter, slow swell — the wide
/// "epic" chord wall for the drop/climax. Held a full bar per chord note (panned wide by chord_spread).
fn supersaw(freq: f32) -> Box<dyn AudioUnit> {
    let saws = (saw_hz(freq)
        + saw_hz(freq * 1.006)
        + saw_hz(freq * 0.994)
        + saw_hz(freq * 1.013)
        + saw_hz(freq * 0.987)
        + saw_hz(freq * 1.020)
        + saw_hz(freq * 0.980))
        * 0.13;
    let cut = envelope(|t: f32| 1300.0 + 3200.0 * (t * 1.0).min(1.0)); // filter swells open
    Box::new(
        // HP off the sub, then DRIVE it (rawstyle screech grit) — a hard wall, not a polite pad.
        ((saws | cut) >> lowpass_q(0.7) >> highpass_hz(180.0, 0.7) >> shape(Tanh(1.8)))
            * envelope(|t: f32| (t * 3.0).min(1.0))
            * 0.42,
    )
}

/// Choir / ensemble pad: a wide bank of detuned saws + a sub-octave sine body through a soft filter
/// with a slow swell — lush grandeur layered UNDER the supersaw wall in the big sections (it carries
/// the warmth/size while the supersaw carries the bright edge). The new diffuse reverb makes it bloom.
fn choir(freq: f32) -> Box<dyn AudioUnit> {
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
fn donk(freq: f32) -> Box<dyn AudioUnit> {
    let saws =
        (saw_hz(freq) + saw_hz(freq * 1.01) + saw_hz(freq * 0.99) + saw_hz(freq * 2.0) * 0.4) * 0.2;
    let cut = envelope(|t: f32| 900.0 + 3600.0 * (-t * 16.0).exp());
    Box::new(
        ((saws | cut) >> lowpass_q(1.0) >> shape(Tanh(1.4)))
            * envelope(|t: f32| {
                let a = 0.003;
                if t < a {
                    t / a
                } else {
                    (-(t - a) * 12.0).exp()
                }
            })
            * 0.4,
    )
}

/// House organ stab: the classic early-90s "M1 organ" rave/house chord stab — Haddaway "What Is Love",
/// Snap!, Cappella. A drawbar-organ tone (fundamental + octave + the nasal fifth + 2-octave partial,
/// like organ drawbars) with a hair of detuned saw for bite, a percussive pluck attack and a short
/// sustain through a bright resonant filter. Hollow + euphoric, but the drive + minor chords keep the
/// dark edge. Rendered per triad note so the chord can be panned wide.
fn houseorg(freq: f32) -> Box<dyn AudioUnit> {
    let organ = (sine_hz(freq)              // 16' fundamental
        + sine_hz(freq * 2.0) * 0.7         // 8'  octave
        + sine_hz(freq * 3.0) * 0.5         // 5⅓' fifth — the nasal organ honk
        + sine_hz(freq * 4.0) * 0.32        // 4'  two octaves up
        + saw_hz(freq * 1.005) * 0.28       // detuned saw pair = the "zaag" bite + width
        + saw_hz(freq * 0.995) * 0.28)
        * 0.17;
    let cut = envelope(|t: f32| 1200.0 + 3600.0 * (-t * 8.5).exp()); // bright pluck that settles fast
    Box::new(
        ((organ | cut) >> lowpass_q(1.1) >> shape(Tanh(1.3)))
            * envelope(|t: f32| {
                let a = 0.004;
                if t < a {
                    t / a
                } else {
                    0.22 + 0.78 * (-(t - a) * 7.0).exp() // percussive attack → a short organ sustain
                }
            })
            * 0.44,
    )
}

/// CASIO / electric-piano: a tine-ish voice (sine carrier + a bell "ting" harmonic + a hair of saw
/// cheese) with a pluck-to-light-sustain envelope — the kitschy Ome-Henk keyboard comping.
fn casio(freq: f32) -> Box<dyn AudioUnit> {
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

fn pseudo_noise(i: usize) -> f32 {
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
fn vel(t: f32, beat: f32, seed: u32) -> f32 {
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
fn groove(t: f32, beat: f32, seed: u32, jit: f32, lay: f32) -> f32 {
    let sl = beat / 4.0;
    let s = (t / sl).round() as i64;
    let swing = if s.rem_euclid(2) == 1 { 0.10 * sl } else { 0.0 };
    let j = pseudo_noise((t * 4099.0) as usize ^ seed as usize) * jit;
    (t + swing + lay + j).max(0.0)
}

/// Ping-pong delay: a stereo delay line where each tap alternates L-R-L-R so the delayed
/// repeats bounce across the stereo field. Used on the arp to give it motion and space.
fn render_pingpong(buf: &mut [f32], delay_s: f32, feedback: f32, wet: f32) {
    let sr = SAMPLE_RATE as f32;
    let d = (delay_s * sr) as usize;
    if d < 2 {
        return;
    }
    let frames = buf.len() / 2;
    let mut line = vec![0f32; d * 2];
    let mut w = 0usize;
    let mut alt = 0u32;
    for i in 0..frames {
        let l = buf[2 * i];
        let r = buf[2 * i + 1];
        let mono = l + r;
        let dl = line[2 * w];
        let dr = line[2 * w + 1];
        buf[2 * i] += dl * wet;
        buf[2 * i + 1] += dr * wet;
        let fb = mono * feedback * 0.5;
        if alt & 1 == 0 {
            line[2 * w] = fb * 0.45;
            line[2 * w + 1] = fb;
        } else {
            line[2 * w] = fb;
            line[2 * w + 1] = fb * 0.45;
        }
        alt = alt.wrapping_add(1);
        w = (w + 1) % d;
    }
}

fn add_stereo(buf: &mut [f32], frame: usize, v: f32, pan: f32) {
    if 2 * frame + 1 >= buf.len() {
        return;
    }
    let (lg, rg) = pan_gains(pan);
    buf[2 * frame] += v * lg;
    buf[2 * frame + 1] += v * rg;
}

/// Noise + tone sweep into a section boundary. This is intentionally simple and deterministic:
/// enough to make the arrangement breathe without turning the score DSL into an effects tracker.
fn render_riser(buf: &mut [f32], start_t: f32, dur: f32, amp: f32, pan: f32) {
    use std::f32::consts::TAU;
    let sr = SAMPLE_RATE as f32;
    let start = (start_t.max(0.0) * sr) as usize;
    let n = (dur.max(0.0) * sr) as usize;
    let mut phase = 0.0f32;
    let mut hp = 0.0f32;
    let denom = std::cmp::max(n, 1) as f32;
    for i in 0..n {
        let p = i as f32 / denom;
        let frame = start + i;
        let hz = 180.0 + 2400.0 * p * p;
        phase = (phase + TAU * hz / sr) % TAU;
        let noise = pseudo_noise(i + start);
        hp += 0.08 * (noise - hp);
        let bright = noise - hp;
        let gate = (p * 16.0).sin().abs() * 0.35 + 0.65;
        let env = p * p * (1.0 - (p - 0.98).max(0.0) * 50.0).clamp(0.0, 1.0);
        add_stereo(
            buf,
            frame,
            (phase.sin() * 0.35 + bright * 0.65) * env * gate * amp,
            pan,
        );
    }
}

/// Atmospheric texture bed under the WHOLE track: a soft band-limited noise floor + sparse vinyl
/// crackle. Game/chiptune music is dead-silent between notes; produced trip-hop/downtempo records
/// (Massive Attack / Portishead) always sit on a dusty textured floor — that bed is a big part of
/// what reads as "a record" instead of "a bright synth preset". Kept low + slightly decorrelated L/R.
fn render_atmosphere(bed: &mut [f32], sr: f32, start_t: f32) {
    use std::f32::consts::TAU;
    let total = bed.len() / 2;
    let start = (start_t.max(0.0) * sr) as usize;
    let fade = (1.5 * sr) as usize; // ease the floor in over ~1.5 s so it doesn't just switch on
    let (mut lp, mut hp) = (0.0f32, 0.0f32);
    let a = 1.0 - (-TAU * 2000.0 / sr).exp();
    let ah = 1.0 - (-TAU * 350.0 / sr).exp();
    for i in start..total {
        let g = ((i - start) as f32 / fade as f32).min(1.0);
        let n = pseudo_noise(i * 2 + 7);
        lp += a * (n - lp); // low-pass...
        hp += ah * (lp - hp); // ...minus a high-pass = a soft ~350-2000 Hz band (warm hiss, no fizz)
        let floor = (lp - hp) * 0.008;
        let crackle = if pseudo_noise(i * 3 + 1) > 0.9996 {
            pseudo_noise(i * 7) * 0.03 // sparser, quieter dust clicks
        } else {
            0.0
        };
        let v = (floor + crackle) * g;
        bed[2 * i] += v;
        bed[2 * i + 1] += v * 0.92;
    }
}

/// Modern hardstyle / rawstyle KICK, tuned per hit to the chord root: a tight click transient → a
/// heavily DISTORTED pitch-swept body (sine + a saw partial driven through tanh then hard-clipped =
/// the "zaag"/gabber grit) → a pitched tonal TAIL on the root pitch-class (the "piep" — the kick is
/// melodic and sings the progression). This is the centre of a modern hard production, not a soft
/// 90s drum-machine thud.
fn render_hardkick(buf: &mut [f32], t: f32, root: f32, amp: f32) {
    use std::f32::consts::TAU;
    let sr = SAMPLE_RATE as f32;
    let start = (t.max(0.0) * sr) as usize;
    let n = (0.5 * sr) as usize;
    // pitch the tonal tail to the root pitch-class in a punchy 55-90 Hz window
    let mut tail_hz = root;
    while tail_hz > 90.0 {
        tail_hz *= 0.5;
    }
    while tail_hz < 55.0 {
        tail_hz *= 2.0;
    }
    let (mut ph_b, mut ph_t) = (0.0f32, 0.0f32);
    for i in 0..n {
        let tt = i as f32 / sr;
        let frame = start + i;
        // body: a fast pitch sweep from ~300 Hz down to the tail pitch over ~13 ms
        let body_hz = tail_hz + (300.0 - tail_hz) * (-tt * 75.0).exp();
        ph_b = (ph_b + TAU * body_hz / sr) % TAU;
        let raw = ph_b.sin() + ((ph_b / TAU) * 2.0 - 1.0) * 0.5; // sine + saw partial (the "zaag")
        let driven = (raw * 5.0).tanh(); // overdrive
        let body = (driven * 1.6).clamp(-1.0, 1.0) * (-tt * 9.0).exp(); // + hard-clip edge, fast decay
                                                                        // tonal tail: the pitched "piep", distorted, slower decay
        ph_t = (ph_t + TAU * tail_hz / sr) % TAU;
        let tail = (ph_t.sin() * 3.0).tanh() * (-tt * 5.0).exp();
        // click transient: bright noise blip for the attack snap
        let click = pseudo_noise(i + start * 11) * (-tt * 300.0).exp() * 0.6;
        add_stereo(buf, frame, (body * 0.95 + tail * 0.45 + click) * amp, 0.0);
    }
}

/// Jet-engine flyby: band-limited noise (a sweeping band-pass built from two one-pole low-passes, so
/// it can't self-oscillate) + a sweeping turbine whine, with a swell-to-flyby-then-away amplitude
/// envelope and a left→right doppler pan. Rips into a section like an afterburner pass.
fn render_jet(buf: &mut [f32], start_t: f32, dur: f32, amp: f32) {
    use std::f32::consts::TAU;
    let sr = SAMPLE_RATE as f32;
    let start = (start_t.max(0.0) * sr) as usize;
    let n = (dur.max(0.0) * sr) as usize;
    let denom = std::cmp::max(n, 1) as f32;
    let (mut lp1, mut lp2, mut lp3) = (0.0f32, 0.0f32, 0.0f32);
    let (mut ph1, mut ph2) = (0.0f32, 0.0f32);
    for i in 0..n {
        let p = i as f32 / denom;
        let frame = start + i;
        let nz = pseudo_noise(i + start * 7);
        // a RESONANT noise band whose centre rises across the pass (faster near the end) — an uplifter
        // "whoosh", not a polite sweep. Two overlapping band-passes stack into a richer scream than the
        // old single 1-pole band did.
        let cut = 350.0 + 3200.0 * p * p;
        let a_lo = 1.0 - (-TAU * cut / sr).exp();
        let a_hi = 1.0 - (-TAU * (cut * 2.2) / sr).exp();
        let a_n = 1.0 - (-TAU * (cut * 1.4) / sr).exp();
        lp1 += a_lo * (nz - lp1);
        lp2 += a_hi * (nz - lp2);
        lp3 += a_n * (nz - lp3);
        let band = (lp2 - lp1) * 2.5 + (lp2 - lp3) * 2.0;
        // a DETUNED-saw turbine pair (not a clean sine — that was the synthetic tell) rising into the
        // hit, low under the noise: pitch motion without the cheesy pure-tone whine.
        let whz = 500.0 + 2200.0 * p;
        ph1 = (ph1 + TAU * whz / sr) % TAU;
        ph2 = (ph2 + TAU * whz * 1.011 / sr) % TAU;
        let saw = |ph: f32| (ph / TAU) * 2.0 - 1.0;
        let turbine = (saw(ph1) + saw(ph2)) * 0.06;
        let env = (1.0 - (2.0 * p - 1.0).abs()).powf(1.3); // swell → flyby → away
        let v = ((band + turbine) * env).tanh() * amp; // soft drive → grit, not a clean sweep
        add_stereo(buf, frame, v, (2.0 * p - 1.0) * 0.8);
    }
}

/// Low boom + short noisy crack at a downbeat.
fn render_impact(buf: &mut [f32], t: f32, dur: f32, amp: f32) {
    use std::f32::consts::TAU;
    let sr = SAMPLE_RATE as f32;
    let start = (t.max(0.0) * sr) as usize;
    let n = (dur.max(0.0) * sr) as usize;
    let mut phase = 0.0f32;
    let denom = std::cmp::max(n, 1) as f32;
    for i in 0..n {
        let p = i as f32 / denom;
        let frame = start + i;
        let hz = 92.0 * (1.0 - p).powf(2.0) + 32.0;
        phase = (phase + TAU * hz / sr) % TAU;
        let boom = phase.sin() * (-p * 4.5).exp();
        let crack = pseudo_noise(i + start * 3) * (-p * 38.0).exp();
        add_stereo(buf, frame, (boom * 0.9 + crack * 0.25) * amp, 0.0);
    }
}

fn section_time(score: &Score, name: &str) -> Option<f32> {
    score
        .sections
        .iter()
        .position(|s| s.name == name)
        .map(|i| score.section_start_secs(i))
}

/// `(start, end)` seconds of a named section (end = next section's start, or the demo end).
fn section_window(score: &Score, name: &str) -> Option<(f32, f32)> {
    let i = score.sections.iter().position(|s| s.name == name)?;
    let start = score.section_start_secs(i);
    let end = if i + 1 < score.sections.len() {
        score.section_start_secs(i + 1)
    } else {
        score.demo_len()
    };
    Some((start, end))
}

/// Accelerating, rising snare roll over `[start, start+dur]` — the build-up tension into a drop.
fn render_snare_roll(buf: &mut [f32], start: f32, dur: f32, beat: f32) {
    let mut t = 0.0;
    let mut step = beat;
    while t < dur {
        let p = (t / dur).clamp(0.0, 1.0);
        render_into(buf, start + t, 0.16, 0.10 + 0.5 * p, 0.0, snare());
        step = (step * 0.86).max(beat * 0.12); // tighten toward the drop
        t += step;
    }
}

fn render_intro_bassline(buf: &mut [f32], score: &Score) {
    let bar = score.bar();
    let beat = score.beat();
    let Some(build_t) = section_time(score, "build") else {
        return;
    };

    let start_bar = 4;
    let end_bar = (build_t / bar).floor() as usize;
    for b in start_bar..end_bar {
        let t = b as f32 * bar;
        let root = bass_freq(score.chord_at(t).root);
        let amp = if b < 6 { 0.18 } else { 0.26 };
        render_into(buf, t, 0.7, amp, 0.0, bass(root, 0.85));
        if b >= 5 {
            render_into(buf, t + 2.0 * beat, 0.45, amp * 0.75, 0.0, bass(root, 0.7));
        }
        if b >= 7 {
            render_into(
                buf,
                t + 3.0 * beat,
                0.35,
                amp * 0.55,
                0.0,
                bass(root * 1.5, 0.6),
            );
        }
    }
}

fn render_intro_percussion(kickbuf: &mut [f32], bed: &mut [f32], score: &Score) {
    let bar = score.bar();
    let beat = score.beat();
    let Some(build_t) = section_time(score, "build") else {
        return;
    };

    let bars = (build_t / bar).floor() as usize;
    for b in 2..bars {
        let base = b as f32 * bar;
        let k_amp = if b < 4 { 0.30 } else { 0.55 };
        let root = score.chord_at(base).root;
        render_hardkick(kickbuf, base, root, k_amp);
        if b >= 4 {
            render_hardkick(kickbuf, base + 2.0 * beat, root, k_amp * 0.55);
        }
        if b >= 5 {
            render_into(bed, base + beat, 0.10, 0.12, -0.35, hat());
            render_into(bed, base + 3.0 * beat, 0.10, 0.12, 0.35, hat());
        }
        if b >= 6 {
            for s in 0..8 {
                render_into(
                    bed,
                    base + s as f32 * beat * 0.5,
                    0.07,
                    0.07,
                    if s % 2 == 0 { -0.45 } else { 0.45 },
                    hat(),
                );
            }
        }
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

/// Keep a chord root in the deep sub range. Score roots are parsed around octave 3; the sub layer
/// wants the same pitch class down around 24-38 Hz, with an added harmonic later for translation on
/// smaller speakers.
fn sub_freq(root: f32) -> f32 {
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
fn bass_freq(root: f32) -> f32 {
    sub_freq(root) * 2.0
}

/// Spread reverb send: a mono sum of the stereo bed through 3 damped feedback combs per channel,
/// with slightly different delays L vs R → a wide, decorrelated room tail (dry excluded).
fn reverb_send(bed: &[f32], sr: f32) -> Vec<f32> {
    let frames = bed.len() / 2;
    let damp = 0.25_f32;
    // mono sum of the bed, HIGH-PASSED at ~300 Hz before the combs so the tail is air/space, not a
    // low-mid wash that welds the voices together (the reverb was a big part of the "organ" blanket).
    let mut mono: Vec<f32> = (0..frames)
        .map(|i| 0.5 * (bed[2 * i] + bed[2 * i + 1]))
        .collect();
    let a = 1.0 - (-std::f32::consts::TAU * 300.0 / sr).exp();
    let mut hp = 0.0f32;
    for s in mono.iter_mut() {
        hp += a * (*s - hp);
        *s -= hp;
    }
    // ~22 ms pre-delay: the gap before the tail that makes the space read as a big, real hall.
    let pre = (0.022 * sr) as usize;
    // 6 prime-length feedback combs per tank — the modes interleave into a smooth dense tail instead
    // of a few resonant metallic rings. Two decorrelated delay sets feed the L and R tanks (width).
    let comb = |delays: &[usize]| -> Vec<f32> {
        let mut wet = vec![0f32; frames];
        for &d in delays {
            let mut line = vec![0f32; frames];
            let mut lp = 0f32;
            for i in 0..frames {
                let src = if i >= pre { mono[i - pre] } else { 0.0 };
                let fb_in = if i >= d { line[i - d] } else { 0.0 };
                lp += damp * (fb_in - lp);
                line[i] = src + 0.88 * lp;
                wet[i] += 0.88 * lp;
            }
        }
        for w in wet.iter_mut() {
            *w *= 0.5; // 6 combs sum hot — tame before diffusion so the wet doesn't pump the limiter
        }
        wet
    };
    // in-place series all-pass diffuser: smears the comb echoes into a smooth, diffuse tail.
    let allpass = |x: &mut [f32], d: usize, g: f32| {
        let mut buf = vec![0f32; x.len()];
        for i in 0..x.len() {
            let dl = if i >= d { buf[i - d] } else { 0.0 };
            let y = -g * x[i] + dl;
            buf[i] = x[i] + g * y;
            x[i] = y;
        }
    };
    let mut wl = comb(&[1117, 1188, 1277, 1356, 1422, 1491]);
    let mut wr = comb(&[1129, 1213, 1291, 1373, 1447, 1499]);
    for &d in &[0.0051f32, 0.0167, 0.0097] {
        allpass(&mut wl, (d * sr) as usize, 0.7);
    }
    for &d in &[0.0047f32, 0.0153, 0.0089] {
        allpass(&mut wr, (d * sr) as usize, 0.7);
    }
    // darken the wet return (~6.5 kHz one-pole LP) so the tail sits behind the mix like a real room.
    let ad = 1.0 - (-std::f32::consts::TAU * 6500.0 / sr).exp();
    let (mut dl, mut dr) = (0.0f32, 0.0f32);
    let mut out = vec![0f32; bed.len()];
    for i in 0..frames {
        dl += ad * (wl[i] - dl);
        dr += ad * (wr[i] - dr);
        out[2 * i] = dl;
        out[2 * i + 1] = dr;
    }
    out
}

/// Render the whole score to an interleaved-stereo buffer: voices panned into a "bed" (everything
/// but the kick), an arp counter-line in the energetic sections, sidechain pump under the kick, a
/// spread reverb send, the continuous sub, per-section fades × gain, soft clip.
pub fn synth_track(score: &Score) -> Track {
    let sr = SAMPLE_RATE as f32;
    let total = (score.demo_len() * sr).ceil() as usize;
    let stereo = total * 2;
    let mut kickbuf = vec![0f32; stereo]; // sidechain source (never ducked)
    let mut bed = vec![0f32; stereo]; // everything else (pumped + reverbed)

    // The render is split into ordered passes — drums → voices → harmony → fx → master. The order is
    // load-bearing: every pass ACCUMULATES into `bed[i] += …`, so keeping the passes in this exact
    // sequence keeps the floating-point sum bit-for-bit identical to the old monolithic function.
    let kicks = score.hits(Inst::Kick);
    render_drums(&mut kickbuf, &mut bed, score, &kicks);
    render_voices(&mut bed, score, stereo);
    render_harmony(&mut bed, score);
    render_fx(&mut bed, score, total);
    let buf = master(&kickbuf, &mut bed, score, &kicks, total, stereo);
    Track {
        samples: Arc::new(buf),
    }
}

/// Drums → the kick (its own buffer, the sidechain source) + a short bass body under it, the intro
/// percussion, and the snare / hat / stab lanes panned across the field. `kicks` is passed in (it is
/// also the sidechain source in `master`, so it's computed once).
fn render_drums(kickbuf: &mut [f32], bed: &mut [f32], score: &Score, kicks: &[f32]) {
    let beat = score.beat();
    // the rhythm section sits forward: hi-hats + snares are level-knobbed so they can be pushed up to
    // read as a prominent groove (the parts coming in were too polite). `set hats=` / `set snares=`.
    let hats_amp = score.param("hats", 0.3);
    let snare_amp = score.param("snares", 0.58);
    for &kt in kicks {
        // kick stays near-full + dead-on so the pump locks; just a hair of velocity for life.
        render_hardkick(
            kickbuf,
            kt,
            score.chord_at(kt).root,
            0.92 * (0.9 + 0.1 * vel(kt, beat, 0)),
        );
        // a SHORT, light bass body under the kick — just transient punch, not a sustained low pile.
        // The pitched kick tail + the continuous sub already carry the low end, so a long/loud bass
        // here only muddies it; keep it brief (0.25s) and quiet (0.18) so it adds thump, not mud.
        let v = vel(kt, beat, 0x88);
        render_into(
            bed,
            kt,
            0.25,
            0.18 * v,
            0.0,
            bass(bass_freq(score.chord_at(kt).root), v),
        );
    }
    render_intro_percussion(kickbuf, bed, score);

    for (i, t) in score.hits(Inst::Snare).into_iter().enumerate() {
        // snares alternate left/centre/right — they're the backbeat anchor, spread them so the
        // groove breathes across the field instead of sitting dead centre.
        let pan = match i % 3 {
            0 => -0.2,
            1 => 0.15,
            _ => -0.05,
        };
        render_into(
            bed,
            groove(t, beat, 0x55, 0.003, 0.004),
            0.4,
            snare_amp * vel(t, beat, 0x55),
            pan,
            snare(),
        );
    }
    for (i, t) in score.hits(Inst::Hat).into_iter().enumerate() {
        let pan = if i % 2 == 0 { 0.65 } else { -0.65 }; // hats dance wider across the field
        render_into(
            bed,
            groove(t, beat, 0x77, 0.006, 0.0),
            0.12,
            hats_amp * vel(t, beat, 0x77),
            pan,
            hat(),
        );
    }
    for t in score.hits(Inst::Stab) {
        let m = score.levels(t).mids;
        chord_spread(
            bed,
            groove(t, beat, 0x6E, 0.004, 0.0),
            0.5,
            (0.10 + 0.10 * m) * vel(t, beat, 0x6E),
            0.75,
            score.chord_at(t).triad(),
            stab,
        );
    }
}

/// Melodic voices → the articulated intro bassline, the forward lead (+ octave sheen + a dotted-8th
/// ping-pong echo added wet-only for depth), the arp counter-line (in its own buffer, ping-ponged),
/// and the `<section>.bass` note-lane. `stereo` sizes the scratch echo/arp buffers.
fn render_voices(bed: &mut [f32], score: &Score, stereo: usize) {
    let beat = score.beat();
    render_intro_bassline(bed, score);

    // lead: forward + centre — the HOOK, push it up so it cuts through the wall. The lead is the
    // melody the viewer hums leaving the tent; it does NOT sit behind the drums.
    let climax = section_window(score, "climax");
    for (t, f) in score.lead_notes() {
        let v = vel(t, beat, 0x1A);
        let gt = groove(t, beat, 0x3A, 0.005, 0.005);
        render_into(
            bed,
            gt,
            0.6,
            score.param_at(t, "lead", 0.82) * v,
            0.0,
            lead(f, v),
        ); // STAR — `set lead=`
        render_into(bed, gt, 0.6, 0.20 * v, 0.0, lead(f * 2.0, v)); // octave sheen
        if let Some((s0, s1)) = climax {
            if (s0..s1).contains(&t) {
                render_into(bed, gt, 0.6, 0.18 * v, 0.0, lead(f * 2.0, v));
                // extra sheen in climax
            }
        }
    }
    // lead depth: a dotted-8th ping-pong of the lead in its own buffer; we add only the WET (echoes)
    // so the dry lead stays up front while its repeats open a 3D space behind it (front-to-back depth).
    let mut lead_echo = vec![0f32; stereo];
    for (t, f) in score.lead_notes() {
        let v = vel(t, beat, 0x1A);
        let gt = groove(t, beat, 0x3A, 0.005, 0.005);
        render_into(
            &mut lead_echo,
            gt,
            0.5,
            0.30 * v,
            0.0,
            lead(f, (v * 0.7).max(0.25)),
        );
    }
    let lead_dry = lead_echo.clone();
    render_pingpong(&mut lead_echo, beat * 0.75, 0.34, 0.55); // dotted-8th throw
    for i in 0..stereo {
        bed[i] += lead_echo[i] - lead_dry[i]; // echoes only — the dry lead already sits in bed
    }

    // arp counter-line into its OWN buffer so we can process it (ping-pong delay) without
    // smearing the drums or the lead — spatial separation is the whole trick.
    let mut arp_buf = vec![0f32; stereo];
    for (i, (t, f)) in score.arp_notes().into_iter().enumerate() {
        let pan = if i % 2 == 0 { 0.7 } else { -0.7 };
        let v = vel(t, beat, 0x2B);
        render_into(
            &mut arp_buf,
            groove(t, beat, 0x9C, 0.006, 0.0),
            0.2,
            0.20 * v,
            pan,
            arp(f, v),
        );
    }
    // ping-pong delay: 8th note, bounces L-R-L-R, 3–4 repeats, glued under the lead. (`beat` is
    // seconds-per-beat, so an 8th note is beat/2 — NOT 60/beat, which would be a ~70 s no-op.)
    render_pingpong(&mut arp_buf, beat / 2.0, 0.35, 0.30);
    for i in 0..stereo {
        bed[i] += arp_buf[i];
    }

    // articulated bassline: the `<section>.bass` note-lane (the real funky bass), centred — a punchy
    // `bass` voice at each onset, riding on top of the continuous drone sub below. `set woozbass=1`
    // swaps in the dark/growl/woozy `woozbass` voice (held a touch longer so its growl can bloom).
    let wooz = score.param("woozbass", 0.0) > 0.5;
    for (t, f) in score.bass_notes() {
        let v = vel(t, beat, 0xB5);
        let amp = (0.20 + 0.18 * score.levels(t).sub_bass) * v; // sit with the section's sub level
        let (dur, voice): (f32, Box<dyn AudioUnit>) = if wooz {
            (0.6, woozbass(f))
        } else {
            (0.42, bass(f, v))
        };
        render_into(bed, groove(t, beat, 0xB5, 0.003, 0.0), dur, amp, 0.0, voice);
    }
}

/// Sustained harmony → the auto-panning pad (every bar), the epic supersaw + choir wall and the
/// octave-up shimmer bloom in the big sections, and the off-beat stab layers (donk / house organ /
/// casio). All gated by section name.
fn render_harmony(bed: &mut [f32], score: &Score) {
    use std::f32::consts::TAU;
    let beat = score.beat();
    let bar = score.bar();
    // sustained pad: one chord per bar, spread wide (warmth/body) with a SLOW auto-pan LFO so the
    // pad breathes and rotates across the stereo field — a static pad reads as wallpaper, a
    // moving one as atmosphere.
    let nbars = (score.demo_len() / bar).ceil() as usize;
    for b in 0..nbars {
        let t = b as f32 * bar;
        let m = score.levels(t).mids;
        let intro_pad = ((t - 6.0 * bar) / (2.0 * bar)).clamp(0.0, 1.0);
        let pan_spread = 0.5 + 0.25 * (t * 0.4 / bar * TAU).sin();
        chord_spread(
            bed,
            t,
            bar,
            (0.06 + 0.10 * m) * intro_pad,
            pan_spread,
            score.chord_at(t).triad(),
            pad,
        );
    }

    // epic supersaw chord wall: wide detuned saws on the chords, one per bar, gated to the BIG
    // sections (drop + climax + the OUTRO finale) so the dynamic range stays — quiet intro/breakdown →
    // wall in the drop. The outro carries the wall so the ending rings out as a full anthem, not a thin
    // lead over drums.
    for name in ["drop", "climax", "outro"] {
        if let Some((s0, s1)) = section_window(score, name) {
            let mut b = (s0 / bar).ceil() as usize;
            while (b as f32) * bar < s1 {
                let t = b as f32 * bar;
                let m = score.levels(t).mids;
                let amp = score.param_at(t, "supersaw", 0.07) + 0.07 * m; // `set supersaw=` — wall level
                                                                          // Width = the big cheap-vs-produced tell: render each triad note as a decorrelated
                                                                          // hard-L / hard-R pair (the R voice detuned +0.4%) instead of one mono chord — a wide
                                                                          // wall, not a centred pile.
                for &f in score.chord_at(t).triad().iter() {
                    render_into(bed, t, bar, amp * 0.7, -0.95, supersaw(f));
                    render_into(bed, t, bar, amp * 0.7, 0.95, supersaw(f * 1.004));
                    // lush choir bed an octave below the wall — grandeur/warmth under the bright saws
                    let ch = score.param_at(t, "choir", 0.5); // `set choir=` — grandeur bed level
                    render_into(bed, t, bar, amp * ch, -0.6, choir(f * 0.5));
                    render_into(bed, t, bar, amp * ch, 0.6, choir(f * 0.5 * 1.003));
                }
                b += 1;
            }
        }
    }

    // SHIMMER: an airy octave-UP choir pad that eases in across the climax AND the outro — the
    // euphoric, almost-angelic top that lifts the final sections into the payoff (they read as weak/
    // thin without an opening "bloom" over them). Additive, wide, and gated to those two sections.
    // (`set shimmer=0` off.)
    let shimmer = score.param("shimmer", 0.09);
    if shimmer > 0.001 {
        for name in ["climax", "outro"] {
            if let Some((s0, s1)) = section_window(score, name) {
                let mut b = (s0 / bar).ceil() as usize;
                while (b as f32) * bar < s1 {
                    let t = b as f32 * bar;
                    let ramp = ((t - s0) / ((s1 - s0) * 0.6)).clamp(0.0, 1.0); // swells in over the first 60%
                    for &f in score.chord_at(t).triad().iter() {
                        render_into(bed, t, bar, shimmer * ramp, -0.85, choir(f * 2.0));
                        render_into(bed, t, bar, shimmer * ramp, 0.85, choir(f * 2.0 * 1.004));
                    }
                    b += 1;
                }
            }
        }
    }

    // euphoric off-beat "DONK" stab — happy-hardcore / house party energy: a bright plucky chord
    // bounce on every up-beat (the "and") through the drop + climax, under the held wall.
    let hb = score.beat() / 2.0;
    for name in ["drop", "climax"] {
        if let Some((s0, s1)) = section_window(score, name) {
            let mut t = (s0 / score.beat()).ceil() * score.beat() + hb; // first up-beat
            while t < s1 {
                let m = score.levels(t).mids;
                chord_spread(
                    bed,
                    groove(t, beat, 0xD0, 0.004, 0.0),
                    hb * 0.9,
                    (score.param_at(t, "donk", 0.055) + 0.05 * m) * vel(t, beat, 0xD0), // `set donk=`
                    0.55,
                    score.chord_at(t).triad(),
                    donk,
                );
                t += score.beat();
            }
        }
    }

    // CLASSIC HOUSE ORGAN STAB — the early-90s "M1 organ" rave/house sound (Haddaway / Snap!): a wide
    // organ chord bouncing on the off-beats through the euphoric majors + the finale. This is the "zaag
    // orgel house" the track was missing; it sits ON TOP of the (now quieter) donk pluck so the off-beat
    // reads as organ + bite, and it rides the minor/major chords so the dark edge survives the happiness.
    for name in ["drop", "climax", "outro"] {
        if let Some((s0, s1)) = section_window(score, name) {
            let mut t = (s0 / score.beat()).ceil() * score.beat() + hb; // first up-beat
            while t < s1 {
                let m = score.levels(t).mids;
                chord_spread(
                    bed,
                    groove(t, beat, 0x40, 0.004, 0.0),
                    hb * 0.95,
                    (score.param_at(t, "house", 0.12) + 0.06 * m) * vel(t, beat, 0x40), // `set house=`
                    0.7,
                    score.chord_at(t).triad(),
                    houseorg,
                );
                t += score.beat();
            }
        }
    }

    // CASIO comp: a cheesy off-beat chord "chnk" on every up-beat (the "and"), gated to the END of
    // the track (climax + outro) where the Ome-Henk electric piano creeps in.
    let half = score.beat() / 2.0;
    for name in ["outro"] {
        if let Some((s0, s1)) = section_window(score, name) {
            let mut t = (s0 / score.beat()).ceil() * score.beat() + half; // first up-beat
            while t < s1 {
                let m = score.levels(t).mids;
                chord_spread(
                    bed,
                    groove(t, beat, 0x4C, 0.005, 0.0),
                    half * 0.95,
                    (0.05 + 0.06 * m) * vel(t, beat, 0x4C),
                    0.5,
                    score.chord_at(t).triad(),
                    casio,
                );
                t += score.beat();
            }
        }
    }
}

/// Section-transition FX (risers, snare-rolls, jet whooshes, impacts), then the continuous sub-bass
/// and the atmosphere bed — the last things added to `bed` before the master. `total` is the mono
/// frame count (the sub + atmosphere run sample-by-sample).
fn render_fx(bed: &mut [f32], score: &Score, total: usize) {
    use std::f32::consts::TAU;
    let sr = SAMPLE_RATE as f32;
    let bar = score.bar();
    if let Some(t) = section_time(score, "build") {
        render_riser(bed, t - 2.0 * bar, 2.0 * bar, 0.10, -0.25);
    }
    if let Some(t) = section_time(score, "drop") {
        render_riser(bed, t - 4.0 * bar, 4.0 * bar, 0.26, 0.15);
        render_snare_roll(bed, t - 2.0 * bar, 2.0 * bar, score.beat()); // build-up roll
        render_jet(bed, t - 3.0 * bar, 3.0 * bar, 0.32); // afterburner pass into the drop
        render_impact(bed, t, 1.6, 0.62);
    }
    if let Some(t) = section_time(score, "breakdown") {
        render_impact(bed, t, 2.2, 0.38);
    }
    if let Some(t) = section_time(score, "climax") {
        render_riser(bed, t - 4.0 * bar, 4.0 * bar, 0.34, -0.15);
        render_jet(bed, t - 4.0 * bar, 4.0 * bar, 0.5); // a screaming jet rips into the climax
        render_impact(bed, t, 2.0, 0.72);
    }
    // EXPLOSIVE finale — but CLEAN. The outro is the anthem: the chorus melody + the full epic wall
    // (extended above) + the crescendo gain carry it, ringing out with power. The FX stay out of the
    // way: ONE impact lands the outro downbeat, then a SINGLE accelerating snare-roll + riser + jet in
    // the LAST few bars builds into ONE massive final blast. (The old version ran a roll + risers + a
    // mid-outro hit across the WHOLE ~40 s outro — that read as messy noise, not a rising, epic finale.)
    if let Some(t0) = section_time(score, "outro") {
        let end = score.demo_len();
        // build OUT of the climax's final phase straight INTO the outro downbeat — a rising riser + roll
        // across the last bars of the climax so ~3:00→3:05 LIFTS into the finale instead of sagging
        // (the old dead "fake-out breather" sat right here). Pairs with the now-driving climax p3.
        render_riser(bed, t0 - 3.0 * bar, 3.0 * bar, 0.30, 0.0);
        render_snare_roll(bed, t0 - 2.0 * bar, 2.0 * bar, score.beat());
        render_impact(bed, t0, 1.4, 0.5); // land the outro downbeat — then let the anthem ring
        let build = (4.0 * bar).min(end - t0 - 0.1).max(0.5); // the final build window only
        let bs = end - build;
        render_snare_roll(bed, bs, build, score.beat()); // accelerating roll INTO the blast
        render_riser(bed, bs, build, 0.42, 0.0); // a rising uplifter under the roll
        render_jet(bed, end - 2.6, 2.0, 0.6); // jet screams down into the hit
        render_impact(bed, end - 1.9, 2.2, 1.0); // THE blast lands + rings
                                                 // a FINAL knal right at the very end: it's caught at its loud transient when the track stops,
                                                 // so the demo ENDS ON A BANG (no gentle fade-out — see the declick-only fade in `master`).
        render_impact(bed, end - 0.45, 1.0, 1.0);
    }

    // Continuous sub-bass (centre) into the bed so it pumps with the sidechain. It follows the
    // chord root with a short glide instead of droning on one fixed A. The fundamental lives low;
    // a quiet octave harmonic keeps the bass readable on systems that cannot reproduce ~25 Hz.
    let dt = 1.0 / sr;
    let glide = 1.0 - (-dt / 0.045).exp();
    let mut phase = 0.0f32;
    let mut sub_hz = sub_freq(score.chord_at(0.0).root);
    for i in 0..total {
        let t = i as f32 * dt;
        sub_hz += (sub_freq(score.chord_at(t).root) - sub_hz) * glide;
        phase = (phase + TAU * sub_hz * dt) % TAU;
        let fundamental = phase.sin();
        let harmonic = (phase * 2.0).sin() * 0.42; // more 2nd harmonic = an EPIC sub that translates
        let third = (phase * 3.0).sin() * 0.16; // a little grit so it reads on small speakers
        let s = (fundamental + harmonic + third)
            * (0.14 + score.param("sub", 0.46) * score.levels(t).sub_bass); // `set sub=`
        bed[2 * i] += s;
        bed[2 * i + 1] += s;
    }

    // atmosphere: a dusty noise floor + sparse crackle — but ONLY from the build onward (it fades in
    // as the demo kicks off). The intro stays CLEAN sub-bass only; the floor would just read as crowd
    // "juich" noise over the bare intro.
    render_atmosphere(bed, sr, section_time(score, "build").unwrap_or(0.0));
}

/// The master chain: build the sidechain duck (from `kicks`), the spread reverb send + its per-section
/// depth automation, the Haas stereo widen (mutates `bed` in place), then the 2-band master loop →
/// the final interleaved stereo buffer (the caller wraps it in a `Track`).
fn master(
    kickbuf: &[f32],
    bed: &mut [f32],
    score: &Score,
    kicks: &[f32],
    total: usize,
    stereo: usize,
) -> Vec<f32> {
    use std::f32::consts::TAU;
    let sr = SAMPLE_RATE as f32;
    let dt = 1.0 / sr;
    // sidechain pump: a fast dip right on each kick recovering over ~0.11s → the dance "breath".
    let mut duck = vec![1.0f32; total];
    let (depth, tau) = (score.param("sidechain", 0.78), 0.085f32); // `set sidechain=` — pump depth
    for &kt in kicks {
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

    let wet = reverb_send(bed, sr);

    // reverb-depth AUTOMATION: instead of one flat wet send, open the space in the sparse/emotional
    // sections (intro/breakdown/outro — size + feeling) and pull it back in the punchy drops (so the
    // kick + wall stay tight and dry). Automating the reverb like this is a big part of what reads as a
    // produced, 3D record vs a flat one. A per-section target, one-pole smoothed so it glides at the
    // boundaries. (`set reverbauto=0` → flat send.)
    let reverbauto = score.param("reverbauto", 1.0);
    let mut rv_env = vec![1.0f32; total];
    {
        let secmul = |name: &str| match name {
            "intro" => 1.5,
            "build" => 1.15,
            "drop" => 0.7,
            "breakdown" => 1.7,
            "climax" => 0.85,
            "outro" => 1.25,
            _ => 1.0,
        };
        let mut target = vec![1.0f32; total];
        for s in &score.sections {
            if let Some((s0, s1)) = section_window(score, &s.name) {
                let i0 = (s0 * sr) as usize;
                let i1 = std::cmp::min((s1 * sr) as usize, total);
                let mul = secmul(&s.name);
                for v in target.iter_mut().take(i1).skip(i0) {
                    *v = mul;
                }
            }
        }
        let sm = 1.0 - (-dt / 0.3).exp(); // ~0.3 s glide between sections
        let mut e = target.first().copied().unwrap_or(1.0);
        for i in 0..total {
            e += (target[i] - e) * sm;
            rv_env[i] = 1.0 + reverbauto * (e - 1.0); // blend toward flat as reverbauto→0
        }
    }

    // Haas-style stereo widen on the lead: a 12 ms offset between L and R channels makes the
    // lead read as a wide stereo presence without audible echo. Apply ONLY to the lead's
    // frequency range (600 Hz–6 kHz) so we don't smear the bass.
    let haas_d = (0.012 * sr) as usize;
    let mut haas_buf = vec![0f32; haas_d];
    let hp_a = 1.0 - (-TAU * 600.0 / sr).exp();
    let lp_a = 1.0 - (-TAU * 6000.0 / sr).exp();
    let mut hp = 0.0f32;
    let mut bp = 0.0f32;
    for i in 0..total {
        let m = 0.5 * (bed[2 * i] + bed[2 * i + 1]);
        hp += hp_a * (m - hp);
        let h = m - hp; // high-passed at 600
                        // band-pass: one-pole low-pass at 6000 after the HP
        bp += lp_a * (h - bp);
        // write the band-passed signal into the delay line, read from it offset by haas_d
        let delayed = if i >= haas_d {
            haas_buf[(i - haas_d) % haas_d]
        } else {
            0.0
        };
        haas_buf[i % haas_d] = bp;
        // add delayed copy to R channel only (Haas: L arrives first, R arrives late → brain
        // hears a wide phantom image anchored on the left side).
        bed[2 * i + 1] += delayed * 0.25;
    }

    // master: a 2-BAND mastering chain instead of a single full-bus tanh (which glazed the whole mix
    // into an organ). Per channel: split at ~160 Hz → keep the sub + kick CLEAN; on the UPPER band
    // only: mid/side WIDEN (lows stay mono-tight), add an HF-AIR exciter (the "crisp" sparkle the
    // track lacked), then gentle saturation. Recombine, then a shared soft peak-LIMITER for loudness
    // (not a tone-shaping waveshaper) keeps L/R centred.
    let demo = score.demo_len();
    let mut buf = vec![0f32; stereo];
    let split_k = 1.0 - (-std::f32::consts::TAU * 160.0 * dt).exp();
    let mut lp = [0.0f32; 2];
    let mut air_lp = [0.0f32; 2];
    let mut gr = 1.0f32;
    let atk = 1.0 - (-1.0 / (0.0006 * sr)).exp(); // fast attack catches the hard-kick/blast transients
    let rel = 1.0 - (-1.0 / (0.12 * sr)).exp();
    // bus GLUE compressor (slow, ~2:1) — pulls every voice under ONE shared envelope so the pile of
    // isolated voices reads as a single cohesive performance, and lifts the body so it's not soft.
    let mut glue = 1.0f32;
    let g_atk = 1.0 - (-1.0 / (0.010 * sr)).exp(); // 10 ms
    let g_rel = 1.0 - (-1.0 / (0.18 * sr)).exp(); // 180 ms
    let mut g_env = 0.0f32;
    // mix/fx knobs read from the score (`set reverb=… widen=… makeup=… ceiling=…`), hoisted out of
    // the per-sample loop — so the master can be tuned in the score file without recompiling.
    let reverb_amt = score.param("reverb", 0.35);
    let widen = score.param("widen", 1.55);
    let makeup = score.param("makeup", 1.18);
    let ceiling = score.param("ceiling", 0.93);
    for i in 0..total {
        let t = i as f32 * dt;
        let fade_in = (t / 1.5).clamp(0.0, 1.0);
        // END ON THE BANG: only a ~25 ms declick at the very end (not a 2 s fade-down), so the final
        // blast rings out at full and the track stops ON the hit instead of gently fading away.
        let fade_out = ((demo - t) / 0.025).clamp(0.0, 1.0);
        let g = fade_in * fade_out * score.gain_at(t);
        // split each channel into a clean low band + an upper band
        let mut lo = [0.0f32; 2];
        let mut hi = [0.0f32; 2];
        for c in 0..2 {
            let x = kickbuf[2 * i + c]
                + bed[2 * i + c] * duck[i]
                + wet[2 * i + c] * reverb_amt * rv_env[i];
            lp[c] += split_k * (x - lp[c]);
            lo[c] = lp[c];
            hi[c] = x - lp[c];
        }
        // mid/side widen the UPPER band only (low end stays mono → translates + stays tight)
        let m = 0.5 * (hi[0] + hi[1]);
        let s = 0.5 * (hi[0] - hi[1]) * widen;
        hi[0] = m + s;
        hi[1] = m - s;
        // HF-air exciter + gentle saturation on the upper band, recombined with the clean lows
        let mut pre = [0.0f32; 2];
        for c in 0..2 {
            air_lp[c] += 0.5 * (hi[c] - air_lp[c]);
            let air = hi[c] - air_lp[c];
            let hi_x = hi[c] + (air * 1.5).tanh() * 0.3;
            let hi_s = (hi_x * 1.25).tanh();
            pre[c] = (lo[c] + hi_s) * g;
        }
        // bus glue: one slow detector for both channels (2:1 above ~0.5), + makeup so it's LOUDER.
        let det = pre[0].abs().max(pre[1].abs());
        g_env += (det - g_env) * if det > g_env { g_atk } else { g_rel };
        let thr = 0.5;
        let gtarget = if g_env > thr {
            (thr / g_env).sqrt()
        } else {
            1.0
        }; // 2:1
        glue += (gtarget - glue) * if gtarget < glue { g_atk } else { g_rel };
        pre[0] *= glue * makeup; // `set makeup=` — louder, glued
        pre[1] *= glue * makeup;
        // shared soft peak-limiter (one gain for both channels → image stays centred)
        let peak = pre[0].abs().max(pre[1].abs());
        let target = if peak > ceiling { ceiling / peak } else { 1.0 };
        if target < gr {
            gr += (target - gr) * atk;
        } else {
            gr += (1.0 - gr) * rel;
        }
        for c in 0..2 {
            buf[2 * i + c] = (pre[c] * gr).clamp(-1.0, 1.0);
        }
    }
    buf
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
