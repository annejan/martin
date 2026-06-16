//! Sound-design one-shots: the section-transition risers / jet whooshes / impacts, the hardstyle
//! kick, and the snare-roll. These are hand-written DSP (sample loops), not FunDSP voices — the
//! voices are in `voices`. Fired as render events by `render::collect_events`. (The continuous
//! effects — ping-pong delay, atmosphere bed, spread reverb — live in `stream` as resumable
//! block-processors, since the streamer is now the sole render engine.)

use super::voices::snare;
use super::{SAMPLE_RATE, add_stereo, pseudo_noise, render_into};

/// Noise + tone sweep into a section boundary. This is intentionally simple and deterministic:
/// enough to make the arrangement breathe without turning the score DSL into an effects tracker.
pub(super) fn render_riser(buf: &mut [f32], start_t: f32, dur: f32, amp: f32, pan: f32) {
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

/// Modern hardstyle / rawstyle KICK, tuned per hit to the chord root: a tight click transient → a
/// heavily DISTORTED pitch-swept body (sine + a saw partial driven through tanh then hard-clipped =
/// the "zaag"/gabber grit) → a pitched tonal TAIL on the root pitch-class (the "piep" — the kick is
/// melodic and sings the progression). This is the centre of a modern hard production, not a soft
/// 90s drum-machine thud.
pub(super) fn render_hardkick(buf: &mut [f32], t: f32, root: f32, amp: f32) {
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
pub(super) fn render_jet(buf: &mut [f32], start_t: f32, dur: f32, amp: f32) {
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
pub(super) fn render_impact(buf: &mut [f32], t: f32, dur: f32, amp: f32) {
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

/// Accelerating, rising snare roll over `[start, start+dur]` — the build-up tension into a drop.
pub(super) fn render_snare_roll(buf: &mut [f32], start: f32, dur: f32, beat: f32) {
    let mut t = 0.0;
    let mut step = beat;
    while t < dur {
        let p = (t / dur).clamp(0.0, 1.0);
        render_into(buf, start + t, 0.16, 0.10 + 0.5 * p, 0.0, snare());
        step = (step * 0.86).max(beat * 0.12); // tighten toward the drop
        t += step;
    }
}
