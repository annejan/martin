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

/// Down-lifter: the INVERSE of `render_riser` — a falling pitch + deflating noise wash, the synthwave
/// "suck-down" as energy drops OUT of a big section into a calm one (drop → breakdown). The pitch
/// sweeps ~2400 → 120 Hz and the wash starts present then fades to nothing. Deterministic.
pub(super) fn render_downlift(buf: &mut [f32], start_t: f32, dur: f32, amp: f32, pan: f32) {
    use std::f32::consts::TAU;
    let sr = SAMPLE_RATE as f32;
    let start = (start_t.max(0.0) * sr) as usize;
    let n = (dur.max(0.0) * sr) as usize;
    let denom = std::cmp::max(n, 1) as f32;
    let mut phase = 0.0f32;
    let mut hp = 0.0f32;
    for i in 0..n {
        let p = i as f32 / denom;
        let frame = start + i;
        let q = 1.0 - p; // falls 1 → 0
        let hz = 120.0 + 2300.0 * q * q; // pitch sweeps DOWN
        phase = (phase + TAU * hz / sr) % TAU;
        let noise = pseudo_noise(i + start);
        hp += 0.08 * (noise - hp);
        let bright = noise - hp;
        let env = (p * 30.0).min(1.0) * q * q; // soft start (no click), long deflating fade
        add_stereo(
            buf,
            frame,
            (phase.sin() * 0.4 + bright * 0.6) * env * amp,
            pan,
        );
    }
}

/// Tonal riser: a MUSICAL lift instead of pure noise — a detuned saw pair glides UP ~an octave from
/// the chord `root` (folded into a low-mid octave) through a one-pole low-pass that opens as it rises,
/// amp swelling in. Harmonic into the boundary — a melodic build into a climax. Deterministic.
pub(super) fn render_tonalriser(
    buf: &mut [f32],
    start_t: f32,
    dur: f32,
    root: f32,
    amp: f32,
    pan: f32,
) {
    use std::f32::consts::TAU;
    let sr = SAMPLE_RATE as f32;
    let start = (start_t.max(0.0) * sr) as usize;
    let n = (dur.max(0.0) * sr) as usize;
    let denom = std::cmp::max(n, 1) as f32;
    let mut base = root.max(1.0); // fold into a musical low-mid octave (110–220 Hz)
    while base > 220.0 {
        base *= 0.5;
    }
    while base < 110.0 {
        base *= 2.0;
    }
    let (mut ph1, mut ph2) = (0.0f32, 0.0f32);
    let mut lp = 0.0f32;
    for i in 0..n {
        let p = i as f32 / denom;
        let frame = start + i;
        let hz = base * (1.0 + p); // glide up ~an octave over the riser
        ph1 = (ph1 + TAU * hz / sr) % TAU;
        ph2 = (ph2 + TAU * hz * 1.005 / sr) % TAU; // a detuned twin for width
        let saw = ((ph1 / TAU) * 2.0 - 1.0) + ((ph2 / TAU) * 2.0 - 1.0);
        let k = 0.02 + 0.5 * p; // low-pass opens as the riser rises
        lp += k * (saw * 0.5 - lp);
        add_stereo(buf, frame, lp * (p * p) * amp, pan); // p² swell-in
    }
}

/// Reverse-swell: an 80s REVERSE-reverb/cymbal — a bright diffuse noise wash that swells SMOOTHLY up
/// then CUTS hard at the boundary (the abrupt stop right on the downbeat is the signature "sucked-in"
/// sound). No pitch sweep (unlike a riser); just a smoothstep swell + a hard gate at the end. The
/// `bright` sparkle is one-pole-smoothed so it reads as diffuse reverb tail, not raw hiss. Deterministic.
pub(super) fn render_reverse(buf: &mut [f32], start_t: f32, dur: f32, amp: f32, pan: f32) {
    let sr = SAMPLE_RATE as f32;
    let start = (start_t.max(0.0) * sr) as usize;
    let n = (dur.max(0.0) * sr) as usize;
    let denom = std::cmp::max(n, 1) as f32;
    let (mut hp, mut sm) = (0.0f32, 0.0f32);
    for i in 0..n {
        let p = i as f32 / denom;
        let frame = start + i;
        let noise = pseudo_noise(i + start * 13);
        hp += 0.05 * (noise - hp);
        let bright = noise - hp; // high-passed sparkle
        sm += 0.4 * (bright - sm); // smoothed → diffuse reverb-tail wash
        let swell = p * p * (3.0 - 2.0 * p); // smoothstep swell up
        let gate = if p > 0.992 { 0.0 } else { 1.0 }; // HARD cut at the boundary (the reverse snap)
        add_stereo(buf, frame, sm * swell * gate * amp, pan);
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
    // Pitch the tonal tail to the root pitch-class, folded into a punchy 45-90 Hz window. The bounds
    // span exactly one octave (90/45 = 2), so EVERY pitch class has a representative inside — unlike a
    // sub-octave window, where G/G#/F# escaped above the top and the kick rang thin on those roots.
    let mut tail_hz = root;
    while tail_hz > 90.0 {
        tail_hz *= 0.5;
    }
    while tail_hz < 45.0 {
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

/// 808/analog-style night-drive KICK (procedural, NOT FunDSP): a punchy but CLEAN thump for the
/// nocturnal-synthwave bed — no longer the rawstyle/gabber unit it was. Layers: a sine BODY that
/// pitch-sweeps fast from ~220 Hz down to the chord-root pitch (the punch), gentle soft-knee
/// SATURATION for analog warmth instead of the old tanh+hard-clip grit, a tight high-passed CLICK for
/// the attack snap, and a long pure-sine SUB tail (~0.6 s) for the deep weight that sidechains so well
/// against the bass. Still tuned to the root pitch-class (folded into a 45-90 Hz octave window so every
/// root has a representative), still dead-on for the sidechain pump.
pub(super) fn render_kick_sw(buf: &mut [f32], t: f32, root: f32, amp: f32) {
    use std::f32::consts::TAU;
    let sr = SAMPLE_RATE as f32;
    let start = (t.max(0.0) * sr) as usize;
    let n = (0.65 * sr) as usize;
    // Tune the kick to the root pitch-class, folded into a punchy 45-90 Hz window. The bounds span
    // exactly one octave (90/45 = 2), so EVERY pitch class has a representative inside.
    let mut tail_hz = root;
    while tail_hz > 90.0 {
        tail_hz *= 0.5;
    }
    while tail_hz < 45.0 {
        tail_hz *= 2.0;
    }
    let (mut ph_b, mut ph_s) = (0.0f32, 0.0f32);
    for i in 0..n {
        let tt = i as f32 / sr;
        let frame = start + i;
        // BODY: a fast pitch sweep from ~220 Hz down to the tuned pitch over ~12 ms — the punch. Pure
        // sine (no saw partial), so it reads round and analog, not as gabber buzz.
        let body_hz = tail_hz + (220.0 - tail_hz) * (-tt * 80.0).exp();
        ph_b = (ph_b + TAU * body_hz / sr) % TAU;
        // soft-knee saturation: tanh at a GENTLE drive warms + glues the transient without the old
        // hard-clip edge (analog warmth, not distortion).
        let body = (ph_b.sin() * 1.5).tanh() * (-tt * 11.0).exp();
        // SUB tail: a clean sine on the tuned pitch with a long decay — the deep night-drive weight.
        ph_s = (ph_s + TAU * tail_hz / sr) % TAU;
        let sub = ph_s.sin() * (-tt * 4.2).exp();
        // CLICK: a tight noise transient for the attack snap (very fast decay so it stays a tick).
        let click = pseudo_noise(i + start * 11) * (-tt * 420.0).exp() * 0.35;
        add_stereo(buf, frame, (body * 0.9 + sub * 0.55 + click) * amp, 0.0);
    }
}
