//! The render passes: how the score's notes/patterns/dynamics become the stereo bed. `synth_track`
//! (in `mod`) calls these in order — drums → voices → harmony → fx → master — each ACCUMULATING into
//! the shared `bed`, so the pass order is load-bearing. The instruments are in `voices`, the
//! sound-design effects in `effects`, and the low-level helpers (render_into / vel / groove / …) in
//! `mod`.

use super::effects::*;
use super::voices::*;
use super::{
    SAMPLE_RATE, bass_freq, chord_spread, groove, render_into, section_time, section_window,
    sub_freq, vel,
};
use crate::score::{Inst, Score};

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

/// Drums → the kick (its own buffer, the sidechain source) + a short bass body under it, the intro
/// percussion, and the snare / hat / stab lanes panned across the field. `kicks` is passed in (it is
/// also the sidechain source in `master`, so it's computed once).
pub(super) fn render_drums(kickbuf: &mut [f32], bed: &mut [f32], score: &Score, kicks: &[f32]) {
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
pub(super) fn render_voices(bed: &mut [f32], score: &Score, stereo: usize) {
    let beat = score.beat();
    // The four lanes below (lead / lead-echo / arp / bass) are independent — each renders into its
    // own buffer on its own thread (this pass dominates the whole-track render time), then they're
    // summed in the original lane order. The lanes' internal note order is unchanged, so the render
    // stays deterministic; the regrouped summation only moves ±1-LSB quantization noise.
    let ovs = super::oversampling();
    let mut lead_buf = vec![0f32; stereo];
    let mut lead_echo = vec![0f32; stereo];
    let mut arp_buf = vec![0f32; stereo];
    let mut bass_buf = vec![0f32; stereo];
    std::thread::scope(|s| {
        // lead: forward + centre — the HOOK, push it up so it cuts through the wall. The lead is
        // the melody the viewer hums leaving the tent; it does NOT sit behind the drums.
        s.spawn(|| {
            super::set_oversampling(ovs);
            let bed = &mut lead_buf[..];
            render_intro_bassline(bed, score);
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
                if let Some((s0, s1)) = climax
                    && (s0..s1).contains(&t)
                {
                    render_into(bed, gt, 0.6, 0.18 * v, 0.0, lead(f * 2.0, v)); // climax sheen
                }
            }
            super::progress_tick();
        });
        // lead depth: a dotted-8th ping-pong of the lead in its own buffer; only the WET (echoes)
        // is added so the dry lead stays up front while its repeats open a 3D space behind it.
        s.spawn(|| {
            super::set_oversampling(ovs);
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
                lead_echo[i] -= lead_dry[i]; // echoes only — the dry lead is the lane above
            }
            super::progress_tick();
        });
        // arp counter-line into its OWN buffer so we can process it (ping-pong delay) without
        // smearing the drums or the lead — spatial separation is the whole trick.
        s.spawn(|| {
            super::set_oversampling(ovs);
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
            // ping-pong delay: 8th note, bounces L-R-L-R, 3–4 repeats, glued under the lead.
            // (`beat` is seconds-per-beat, so an 8th note is beat/2 — NOT 60/beat.)
            render_pingpong(&mut arp_buf, beat / 2.0, 0.35, 0.30);
            super::progress_tick();
        });
        // articulated bassline: the `<section>.bass` note-lane (the real funky bass), centred — a
        // punchy `bass` voice at each onset, riding on top of the continuous drone sub below.
        // `set woozbass=1` swaps in the dark/growl `woozbass` voice (held longer so it can bloom).
        super::set_oversampling(ovs); // (scope's own thread — flag already set, keep it explicit)
        let wooz = score.param("woozbass", 0.0) > 0.5;
        for (t, f) in score.bass_notes() {
            let v = vel(t, beat, 0xB5);
            let amp = (0.20 + 0.18 * score.levels(t).sub_bass) * v; // section's sub level
            let (dur, voice): (f32, Box<dyn fundsp::prelude32::AudioUnit>) = if wooz {
                (0.6, woozbass(f))
            } else {
                (0.42, bass(f, v))
            };
            render_into(
                &mut bass_buf,
                groove(t, beat, 0xB5, 0.003, 0.0),
                dur,
                amp,
                0.0,
                voice,
            );
        }
        super::progress_tick();
    });
    // sum in the original lane order: intro-bass+lead, echo wet, arp, bass.
    for i in 0..stereo {
        bed[i] += ((lead_buf[i] + lead_echo[i]) + arp_buf[i]) + bass_buf[i];
    }
}

/// Sustained harmony → the auto-panning pad (every bar), the epic supersaw + choir wall and the
/// octave-up shimmer bloom, and the off-beat stab layers (donk / house organ / casio). Each layer is
/// gated by the section's FX set (`Section::fx_on`), defaulting to the names the synth used to wire.
pub(super) fn render_harmony(bed: &mut [f32], score: &Score) {
    use std::f32::consts::TAU;
    let beat = score.beat();
    let bar = score.bar();
    // Like `render_voices`: the six layers (pad / wall / shimmer / donk / house / casio) are
    // independent, so the heavy ones get their own thread + buffer and are summed in the original
    // layer order afterwards (the wall — 12 supersaw/choir voices per bar — dominates this pass).
    let stereo = bed.len();
    let ovs = super::oversampling();
    let mut pad_buf = vec![0f32; stereo];
    let mut wall_buf = vec![0f32; stereo];
    let mut shim_buf = vec![0f32; stereo];
    std::thread::scope(|s| {
        // sustained pad: one chord per bar, spread wide (warmth/body) with a SLOW auto-pan LFO so
        // the pad breathes and rotates across the stereo field — a static pad reads as wallpaper,
        // a moving one as atmosphere.
        s.spawn(|| {
            super::set_oversampling(ovs);
            let nbars = (score.demo_len() / bar).ceil() as usize;
            for b in 0..nbars {
                let t = b as f32 * bar;
                let m = score.levels(t).mids;
                let intro_pad = ((t - 6.0 * bar) / (2.0 * bar)).clamp(0.0, 1.0);
                let pan_spread = 0.5 + 0.25 * (t * 0.4 / bar * TAU).sin();
                chord_spread(
                    &mut pad_buf,
                    t,
                    bar,
                    (0.06 + 0.10 * m) * intro_pad,
                    pan_spread,
                    score.chord_at(t).triad(),
                    pad,
                );
            }
            super::progress_tick();
        });
        // epic supersaw chord wall: wide detuned saws on the chords, one per bar, in the sections
        // whose fx include `wall` (the big drop/climax + the OUTRO finale by default), so the
        // dynamic range stays — quiet intro/breakdown → wall in the drop, anthem in the outro.
        s.spawn(|| {
            super::set_oversampling(ovs);
            for sec in &score.sections {
                if !sec.fx_on("wall") {
                    continue;
                }
                if let Some((s0, s1)) = section_window(score, &sec.name) {
                    let mut b = (s0 / bar).ceil() as usize;
                    while (b as f32) * bar < s1 {
                        let t = b as f32 * bar;
                        let m = score.levels(t).mids;
                        let amp = score.param_at(t, "supersaw", 0.07) + 0.07 * m; // `set supersaw=`
                        // Width = the big cheap-vs-produced tell: render each triad note as a
                        // decorrelated hard-L / hard-R pair (the R voice detuned +0.4%) instead of
                        // one mono chord — a wide wall, not a centred pile.
                        for &f in score.chord_at(t).triad().iter() {
                            render_into(&mut wall_buf, t, bar, amp * 0.7, -0.95, supersaw(f));
                            render_into(
                                &mut wall_buf,
                                t,
                                bar,
                                amp * 0.7,
                                0.95,
                                supersaw(f * 1.004),
                            );
                            // lush choir bed an octave below the wall — grandeur under the saws
                            let ch = score.param_at(t, "choir", 0.5); // `set choir=` — bed level
                            render_into(&mut wall_buf, t, bar, amp * ch, -0.6, choir(f * 0.5));
                            render_into(
                                &mut wall_buf,
                                t,
                                bar,
                                amp * ch,
                                0.6,
                                choir(f * 0.5 * 1.003),
                            );
                        }
                        b += 1;
                    }
                }
            }
            super::progress_tick();
        });
        // SHIMMER: an airy octave-UP choir pad that eases in across the sections whose fx include
        // `shimmer` (climax + outro by default) — the euphoric top that lifts the payoff sections
        // (they read as weak/thin without an opening "bloom"). (`set shimmer=0` off.)
        s.spawn(|| {
            super::set_oversampling(ovs);
            let shimmer = score.param("shimmer", 0.09);
            if shimmer <= 0.001 {
                return;
            }
            for sec in &score.sections {
                if !sec.fx_on("shimmer") {
                    continue;
                }
                if let Some((s0, s1)) = section_window(score, &sec.name) {
                    let mut b = (s0 / bar).ceil() as usize;
                    while (b as f32) * bar < s1 {
                        let t = b as f32 * bar;
                        let ramp = ((t - s0) / ((s1 - s0) * 0.6)).clamp(0.0, 1.0); // 60% swell
                        for &f in score.chord_at(t).triad().iter() {
                            render_into(
                                &mut shim_buf,
                                t,
                                bar,
                                shimmer * ramp,
                                -0.85,
                                choir(f * 2.0),
                            );
                            render_into(
                                &mut shim_buf,
                                t,
                                bar,
                                shimmer * ramp,
                                0.85,
                                choir(f * 2.0 * 1.004),
                            );
                        }
                        b += 1;
                    }
                }
            }
            super::progress_tick();
        });

        // the off-beat stab layers (light) stay on this thread, straight into `bed` — they're the
        // LAST layers in the original order, so summing the threaded buffers in front keeps the
        // per-sample addition order intact.
        super::set_oversampling(ovs);
        // euphoric off-beat "DONK" stab — happy-hardcore party energy: a bright plucky chord bounce
        // on every up-beat (the "and"), in the sections whose fx include `donk`, under the wall.
        let hb = score.beat() / 2.0;
        for sec in &score.sections {
            if !sec.fx_on("donk") {
                continue;
            }
            if let Some((s0, s1)) = section_window(score, &sec.name) {
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
        // CLASSIC HOUSE ORGAN STAB — the early-90s "M1 organ" rave/house sound (Haddaway / Snap!):
        // a wide organ chord bouncing on the off-beats in the sections whose fx include `house`.
        // It sits ON TOP of the (quieter) donk pluck so the off-beat reads as organ + bite, and it
        // rides the minor/major chords so the dark edge survives the happiness.
        for sec in &score.sections {
            if !sec.fx_on("house") {
                continue;
            }
            if let Some((s0, s1)) = section_window(score, &sec.name) {
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
        // CASIO comp: a cheesy off-beat chord "chnk" on every up-beat, in the sections whose fx
        // include `casio` (the outro by default) where the Ome-Henk electric piano creeps in.
        let half = score.beat() / 2.0;
        for sec in &score.sections {
            if !sec.fx_on("casio") {
                continue;
            }
            if let Some((s0, s1)) = section_window(score, &sec.name) {
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
        super::progress_tick();
    });
    // Sum the threaded layers. Note the regrouping: the stabs accumulated into `bed` during the
    // scope, so per-sample this adds (pad+wall+shimmer) AFTER them instead of before — float
    // addition isn't associative, so this moves ±1-LSB quantization noise vs the old sequential
    // layer order (same accepted trade-off as the pass-level parallelism in `synth_track`).
    for i in 0..stereo {
        bed[i] += (pad_buf[i] + wall_buf[i]) + shim_buf[i];
    }
}

/// Section-transition FX (risers, snare-rolls, jet whooshes, impacts), then the continuous sub-bass
/// and the atmosphere bed — the last things added to `bed` before the master. `total` is the mono
/// frame count (the sub + atmosphere run sample-by-sample).
pub(super) fn render_fx(bed: &mut [f32], score: &Score, total: usize) {
    use std::f32::consts::TAU;
    let sr = SAMPLE_RATE as f32;
    let bar = score.bar();
    // Each transition accent is gated on the section's FX set (`riser`/`jet`/`impact`/`bang`) — by
    // default the same sections the synth used to hard-code, but a `<section>.fx:` line overrides it
    // (so e.g. a tropical drop can keep the wall but skip the demoscene jet).
    if let Some(t) = section_time(score, "build")
        && score.fx_on("build", "riser")
    {
        render_riser(bed, t - 2.0 * bar, 2.0 * bar, 0.10, -0.25);
    }
    if let Some(t) = section_time(score, "drop") {
        if score.fx_on("drop", "riser") {
            render_riser(bed, t - 4.0 * bar, 4.0 * bar, 0.26, 0.15);
            render_snare_roll(bed, t - 2.0 * bar, 2.0 * bar, score.beat()); // build-up roll
        }
        if score.fx_on("drop", "jet") {
            render_jet(bed, t - 3.0 * bar, 3.0 * bar, 0.32); // afterburner pass into the drop
        }
        if score.fx_on("drop", "impact") {
            render_impact(bed, t, 1.6, 0.62);
        }
    }
    if let Some(t) = section_time(score, "breakdown")
        && score.fx_on("breakdown", "impact")
    {
        render_impact(bed, t, 2.2, 0.38);
    }
    if let Some(t) = section_time(score, "climax") {
        if score.fx_on("climax", "riser") {
            render_riser(bed, t - 4.0 * bar, 4.0 * bar, 0.34, -0.15);
        }
        if score.fx_on("climax", "jet") {
            render_jet(bed, t - 4.0 * bar, 4.0 * bar, 0.5); // a screaming jet rips into the climax
        }
        if score.fx_on("climax", "impact") {
            render_impact(bed, t, 2.0, 0.72);
        }
    }
    // EXPLOSIVE finale — but CLEAN. The outro is the anthem: the chorus melody + the full epic wall
    // (extended above) + the crescendo gain carry it, ringing out with power. The FX stay out of the
    // way: ONE impact lands the outro downbeat, then a SINGLE accelerating snare-roll + riser + jet in
    // the LAST few bars builds into ONE massive final blast.
    if let Some(t0) = section_time(score, "outro") {
        let end = score.demo_len();
        // build OUT of the climax's final phase straight INTO the outro downbeat — a rising riser + roll
        // across the last bars of the climax so ~3:00→3:05 LIFTS into the finale instead of sagging.
        if score.fx_on("outro", "riser") {
            render_riser(bed, t0 - 3.0 * bar, 3.0 * bar, 0.30, 0.0);
            render_snare_roll(bed, t0 - 2.0 * bar, 2.0 * bar, score.beat());
        }
        // the explosive finale: a downbeat hit, a final build, and the BANG. Gated on `bang` so a
        // gentler genre (no `bang` in its outro fx) just lets the anthem ring out + fade.
        if score.fx_on("outro", "bang") {
            render_impact(bed, t0, 1.4, 0.5); // land the outro downbeat — then let the anthem ring
            let build = (4.0 * bar).min(end - t0 - 0.1).max(0.5); // the final build window only
            let bs = end - build;
            render_snare_roll(bed, bs, build, score.beat()); // accelerating roll INTO the blast
            render_riser(bed, bs, build, 0.42, 0.0); // a rising uplifter under the roll
            render_jet(bed, end - 2.6, 2.0, 0.6); // jet screams down into the hit
            render_impact(bed, end - 1.9, 2.2, 1.0); // THE blast lands + rings
            // a FINAL knal right at the very end: caught at its loud transient when the track stops,
            // so the demo ENDS ON A BANG (no gentle fade-out — see the declick-only fade in `master`).
            render_impact(bed, end - 0.45, 1.0, 1.0);
        }
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
    render_atmosphere(
        bed,
        sr,
        section_time(score, "build").unwrap_or(0.0),
        score.param("atmosphere", 1.0), // `set atmosphere=` — dusty noise-floor level (0 = off)
    );
}

/// The master chain: build the sidechain duck (from `kicks`), the spread reverb send + its per-section
/// depth automation, the Haas stereo widen (mutates `bed` in place), then the 2-band master loop →
/// the final interleaved stereo buffer (the caller wraps it in a `Track`).
pub(super) fn master(
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
