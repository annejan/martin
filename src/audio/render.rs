//! Event generation: how the score's notes/patterns/dynamics become the time-ordered render events
//! that `stream::produce` plays back (the sole engine, for both live playback and batch recordings).
//! `collect_events` buckets every note/hit/accent into its lane as a `(t, closure)` Event; the
//! closures call the instruments in `voices` + the sound-design effects in `effects`, via the
//! low-level helpers (render_into / vel / groove / …) in `mod`. Also home to the two per-frame
//! automation envelopes (`sidechain_duck` / `reverb_env`) the producer's master chain consumes.

use super::effects::*;
use super::voices::*;
use super::{
    SAMPLE_RATE, bass_freq, chord_spread, groove, render_into, section_time, section_window, vel,
};
use crate::score::{Inst, Score};

/// Sidechain duck envelope (one value per mono frame): a fast dip to `1-depth` right on each kick,
/// recovering over ~0.11 s — the dance "breath". `set sidechain=` scales the depth. Built once and
/// applied by the master chain inside the sole engine, `stream::produce`.
pub(super) fn sidechain_duck(score: &Score, kicks: &[f32], total: usize) -> Vec<f32> {
    let sr = SAMPLE_RATE as f32;
    let mut duck = vec![1.0f32; total];
    let (depth, tau) = (score.param("sidechain", 0.78), 0.085f32);
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
    duck
}

/// Reverb-depth automation envelope (one value per mono frame): a per-section wet multiplier — open
/// in the sparse/emotional sections (intro/breakdown/outro), tight in the punchy drops — one-pole
/// smoothed (~0.3 s glide) so it glides at section boundaries. `set reverbauto=0` → flat (all 1.0).
/// Automating the reverb like this is a big part of what reads as a produced, 3D record vs a flat
/// one. Built once and applied by the master chain inside the sole engine, `stream::produce`.
pub(super) fn reverb_env(score: &Score, total: usize) -> Vec<f32> {
    let sr = SAMPLE_RATE as f32;
    let reverbauto = score.param("reverbauto", 1.0);
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
    let dt = 1.0 / sr;
    let sm = 1.0 - (-dt / 0.3).exp(); // ~0.3 s glide between sections
    let mut rv_env = vec![1.0f32; total];
    let mut e = target.first().copied().unwrap_or(1.0);
    for i in 0..total {
        e += (target[i] - e) * sm;
        rv_env[i] = 1.0 + reverbauto * (e - 1.0); // blend toward flat as reverbauto→0
    }
    rv_env
}

/// The master chain: build the sidechain duck (from `kicks`), the spread reverb send + its per-section
/// depth automation, the Haas stereo widen (mutates `bed` in place), then the 2-band master loop →
/// the final interleaved stereo buffer (the caller wraps it in a `Track`).
// ============================================================================================
// EVENT GENERATION — collect the score into time-ordered events (one per note/hit/accent), which
// `stream::produce` renders segment-by-segment. This is the SOLE note-generation path: both live
// playback and batch recordings (`synth_track`) run `produce`, so there is no second copy to keep
// in sync. The continuous layers (sub, atmosphere, the two ping-pong delays, reverb, master) are
// NOT events — they're the resumable finishers in `stream::produce`.
// ============================================================================================
use crate::audio::stream::{
    Event, L_ARP, L_BASS, L_DRUMS, L_ECHO, L_FX, L_LEAD, L_PAD, L_SHIM, L_STABS, L_WALL, LANES,
};

fn ev(lane: &mut Vec<Event>, t: f32, r: impl FnOnce(&mut [f32], &mut [f32]) + Send + 'static) {
    lane.push(Event {
        t,
        render: Box::new(r),
    });
}

// --- PALETTE PICKERS: each `*sw` score knob (>0.5) swaps the nocturnal SYNTHWAVE voice in; default 0
// keeps the original bouncy/hard voices, so e.g. the Camping show is byte-for-byte unchanged. The
// selected bool is captured (Copy) into the per-event render closures.
type Unit = Box<dyn fundsp::prelude32::AudioUnit>;
// leads also have several CHARACTERS: leadsw = 0 orig · 1 *_sw vocal-saw · 2 breathy sung ·
// 3 brass-cut (Carpenter Brut) · 4 FM bell (DX7).
fn lead_pick(n: i32, f: f32, v: f32) -> Unit {
    match n {
        1 => lead_sw(f, v),
        2 => lead_sw2(f, v),
        3 => lead_sw3(f, v),
        4 => lead_sw4(f, v),
        _ => lead(f, v),
    }
}
// bass also has CHARACTERS: basssw = 0 orig · 1 *_sw clean-split Reese · 2 Kavinsky clean sub ·
// 3 Brut Reese growl · 4 acid/303 squelch. (held-note variant = the matching woozbass.)
fn bass_pick(n: i32, f: f32, v: f32) -> Unit {
    match n {
        1 => bass_sw(f, v),
        2 => bass_sw2(f, v),
        3 => bass_sw3(f, v),
        4 => bass_sw4(f, v),
        _ => bass(f, v),
    }
}
fn woozbass_pick(n: i32, f: f32) -> Unit {
    match n {
        1 => woozbass_sw(f),
        2 => woozbass_sw2(f),
        3 => woozbass_sw3(f),
        4 => woozbass_sw4(f),
        _ => woozbass(f),
    }
}
// pads have several CHARACTERS to audition: padsw = 0 orig · 1 the *_sw pair · 2 Juno poly ·
// 3 PWM string-machine · 4 dark cinematic wash.
fn supersaw_pick(n: i32, f: f32) -> Unit {
    match n {
        1 => supersaw_sw(f),
        2 => supersaw_sw2(f),
        3 => supersaw_sw3(f),
        4 => supersaw_sw4(f),
        _ => supersaw(f),
    }
}
fn choir_pick(n: i32, f: f32) -> Unit {
    match n {
        1 => choir_sw(f),
        2 => choir_sw2(f),
        3 => choir_sw3(f),
        4 => choir_sw4(f),
        _ => choir(f),
    }
}
fn snare_pick(sw: bool) -> Unit {
    if sw { snare_sw() } else { snare() }
}
fn hat_pick(sw: bool) -> Unit {
    if sw { hat_sw() } else { hat() }
}
fn kick_pick(sw: bool, buf: &mut [f32], t: f32, root: f32, amp: f32) {
    if sw {
        render_kick_sw(buf, t, root, amp)
    } else {
        render_hardkick(buf, t, root, amp)
    }
}

pub(super) fn collect_events(score: &Score) -> [Vec<Event>; LANES] {
    let mut lanes: [Vec<Event>; LANES] = std::array::from_fn(|_| Vec::new());
    let beat = score.beat();
    let bar = score.bar();
    // palette selectors (default 0 = the original voices; a score `set leadsw=1` etc. opts into the
    // nocturnal-synthwave alternates). Resolved once, captured into the per-event closures below.
    let leadsw = score.param("leadsw", 0.0).round() as i32;
    let basssw = score.param("basssw", 0.0).round() as i32;
    let padsw = score.param("padsw", 0.0).round() as i32;
    let drumsw = score.param("drumsw", 0.0) > 0.5;

    // ---- DRUMS (kick → kickbuf; bass-body/intro-perc/snare/hat/stab → bed) ----
    let hats_amp = score.param("hats", 0.3);
    let snare_amp = score.param("snares", 0.58);
    for kt in score.hits(Inst::Kick) {
        let root = score.chord_at(kt).root;
        let kamp = 0.92 * (0.9 + 0.1 * vel(kt, beat, 0));
        ev(&mut lanes[L_DRUMS], kt, move |_b, k| {
            kick_pick(drumsw, k, kt, root, kamp)
        });
        let v = vel(kt, beat, 0x88);
        let bf = bass_freq(score.chord_at(kt).root);
        ev(&mut lanes[L_DRUMS], kt, move |b, _| {
            render_into(b, kt, 0.25, 0.18 * v, 0.0, bass_pick(basssw, bf, v))
        });
    }
    // intro percussion (hardkick → kickbuf, hats → bed), bars 2..build
    if let Some(build_t) = section_time(score, "build") {
        let bars = (build_t / bar).floor() as usize;
        for b in 2..bars {
            let base = b as f32 * bar;
            let k_amp = if b < 4 { 0.30 } else { 0.55 };
            let root = score.chord_at(base).root;
            ev(&mut lanes[L_DRUMS], base, move |_b, k| {
                kick_pick(drumsw, k, base, root, k_amp)
            });
            if b >= 4 {
                ev(&mut lanes[L_DRUMS], base + 2.0 * beat, move |_b, k| {
                    kick_pick(drumsw, k, base + 2.0 * beat, root, k_amp * 0.55)
                });
            }
            if b >= 5 {
                ev(&mut lanes[L_DRUMS], base + beat, move |b2, _| {
                    render_into(b2, base + beat, 0.10, 0.12, -0.35, hat_pick(drumsw))
                });
                ev(&mut lanes[L_DRUMS], base + 3.0 * beat, move |b2, _| {
                    render_into(b2, base + 3.0 * beat, 0.10, 0.12, 0.35, hat_pick(drumsw))
                });
            }
            if b >= 6 {
                for s in 0..8 {
                    let st = base + s as f32 * beat * 0.5;
                    let pan = if s % 2 == 0 { -0.45 } else { 0.45 };
                    ev(&mut lanes[L_DRUMS], st, move |b2, _| {
                        render_into(b2, st, 0.07, 0.07, pan, hat_pick(drumsw))
                    });
                }
            }
        }
    }
    for (i, t) in score.hits(Inst::Snare).into_iter().enumerate() {
        let pan = match i % 3 {
            0 => -0.2,
            1 => 0.15,
            _ => -0.05,
        };
        let gt = groove(t, beat, 0x55, 0.003, 0.004);
        let amp = snare_amp * vel(t, beat, 0x55);
        ev(&mut lanes[L_DRUMS], gt, move |b, _| {
            render_into(b, gt, 0.4, amp, pan, snare_pick(drumsw))
        });
    }
    for (i, t) in score.hits(Inst::Hat).into_iter().enumerate() {
        let pan = if i % 2 == 0 { 0.65 } else { -0.65 };
        let gt = groove(t, beat, 0x77, 0.006, 0.0);
        let amp = hats_amp * vel(t, beat, 0x77);
        ev(&mut lanes[L_DRUMS], gt, move |b, _| {
            render_into(b, gt, 0.12, amp, pan, hat_pick(drumsw))
        });
    }
    for t in score.hits(Inst::Stab) {
        let m = score.levels(t).mids;
        let gt = groove(t, beat, 0x6E, 0.004, 0.0);
        let amp = (0.10 + 0.10 * m) * vel(t, beat, 0x6E);
        let tri = score.chord_at(t).triad();
        ev(&mut lanes[L_DRUMS], gt, move |b, _| {
            chord_spread(b, gt, 0.5, amp, 0.75, tri, stab)
        });
    }

    // ---- LEAD lane: intro bassline + the forward lead (+ octave/climax sheen) ----
    if let Some(build_t) = section_time(score, "build") {
        let end_bar = (build_t / bar).floor() as usize;
        for b in 4..end_bar {
            let t = b as f32 * bar;
            let root = bass_freq(score.chord_at(t).root);
            let amp = if b < 6 { 0.18 } else { 0.26 };
            ev(&mut lanes[L_LEAD], t, move |bd, _| {
                render_into(bd, t, 0.7, amp, 0.0, bass_pick(basssw, root, 0.85))
            });
            if b >= 5 {
                let t2 = t + 2.0 * beat;
                ev(&mut lanes[L_LEAD], t2, move |bd, _| {
                    render_into(bd, t2, 0.45, amp * 0.75, 0.0, bass_pick(basssw, root, 0.7))
                });
            }
            if b >= 7 {
                let t3 = t + 3.0 * beat;
                ev(&mut lanes[L_LEAD], t3, move |bd, _| {
                    render_into(
                        bd,
                        t3,
                        0.35,
                        amp * 0.55,
                        0.0,
                        bass_pick(basssw, root * 1.5, 0.6),
                    )
                });
            }
        }
    }
    let climax = section_window(score, "climax");
    for (t, f, hold) in score.lead_notes() {
        let v = vel(t, beat, 0x1A);
        let gt = groove(t, beat, 0x3A, 0.005, 0.005);
        let lamp = score.param_at(t, "lead", 0.82) * v;
        let dur = 0.6 + hold; // a `-`/`_` tie holds the note past its slot; hold==0 = unchanged
        let in_climax = climax
            .map(|(s0, s1)| (s0..s1).contains(&t))
            .unwrap_or(false);
        ev(&mut lanes[L_LEAD], gt, move |bd, _| {
            render_into(bd, gt, dur, lamp, 0.0, lead_pick(leadsw, f, v));
            render_into(bd, gt, dur, 0.20 * v, 0.0, lead_pick(leadsw, f * 2.0, v));
            if in_climax {
                render_into(bd, gt, dur, 0.18 * v, 0.0, lead_pick(leadsw, f * 2.0, v));
            }
        });
    }

    // ---- ECHO lane (lead echo source; ping-pong + dry-subtract happens in produce) ----
    for (t, f, hold) in score.lead_notes() {
        let v = vel(t, beat, 0x1A);
        let gt = groove(t, beat, 0x3A, 0.005, 0.005);
        let amp = 0.30 * v;
        let lv = (v * 0.7).max(0.25);
        let dur = 0.5 + hold;
        ev(&mut lanes[L_ECHO], gt, move |bd, _| {
            render_into(bd, gt, dur, amp, 0.0, lead_pick(leadsw, f, lv))
        });
    }

    // ---- ARP lane (ping-pong happens in produce) ----
    for (i, (t, f, hold)) in score.arp_notes().into_iter().enumerate() {
        let pan = if i % 2 == 0 { 0.7 } else { -0.7 };
        let v = vel(t, beat, 0x2B);
        let gt = groove(t, beat, 0x9C, 0.006, 0.0);
        let dur = 0.2 + hold;
        ev(&mut lanes[L_ARP], gt, move |bd, _| {
            render_into(bd, gt, dur, 0.20 * v, pan, arp(f, v))
        });
    }

    // ---- BASS lane: the articulated `<section>.bass` note-lane ----
    let wooz = score.param("woozbass", 0.0) > 0.5;
    for (t, f, hold) in score.bass_notes() {
        let v = vel(t, beat, 0xB5);
        let amp = (0.20 + 0.18 * score.levels(t).sub_bass) * v;
        let gt = groove(t, beat, 0xB5, 0.003, 0.0);
        ev(&mut lanes[L_BASS], gt, move |bd, _| {
            let (dur, voice): (f32, Box<dyn fundsp::prelude32::AudioUnit>) = if wooz {
                (0.6 + hold, woozbass_pick(basssw, f))
            } else {
                (0.42 + hold, bass_pick(basssw, f, v))
            };
            render_into(bd, gt, dur, amp, 0.0, voice)
        });
    }

    // ---- PAD lane: one sustained chord per bar ----
    let nbars = (score.demo_len() / bar).ceil() as usize;
    for b in 0..nbars {
        let t = b as f32 * bar;
        let m = score.levels(t).mids;
        let intro_pad = ((t - 6.0 * bar) / (2.0 * bar)).clamp(0.0, 1.0);
        let amp = (0.06 + 0.10 * m) * intro_pad;
        let pan_spread = 0.5 + 0.25 * (t * 0.4 / bar * std::f32::consts::TAU).sin();
        let tri = score.chord_at(t).triad();
        ev(&mut lanes[L_PAD], t, move |bd, _| {
            chord_spread(bd, t, bar, amp, pan_spread, tri, pad)
        });
    }

    // ---- WALL lane: supersaw + choir wall, sections with fx `wall` ----
    for sec in &score.sections {
        if !sec.fx_on("wall") {
            continue;
        }
        if let Some((s0, s1)) = section_window(score, &sec.name) {
            let mut b = (s0 / bar).ceil() as usize;
            while (b as f32) * bar < s1 {
                let t = b as f32 * bar;
                let m = score.levels(t).mids;
                let amp = score.param_at(t, "supersaw", 0.07) + 0.07 * m;
                let ch = score.param_at(t, "choir", 0.5);
                let tri = score.chord_at(t).triad();
                ev(&mut lanes[L_WALL], t, move |bd, _| {
                    for &f in tri.iter() {
                        render_into(bd, t, bar, amp * 0.7, -0.95, supersaw_pick(padsw, f));
                        render_into(bd, t, bar, amp * 0.7, 0.95, supersaw_pick(padsw, f * 1.004));
                        render_into(bd, t, bar, amp * ch, -0.6, choir_pick(padsw, f * 0.5));
                        render_into(
                            bd,
                            t,
                            bar,
                            amp * ch,
                            0.6,
                            choir_pick(padsw, f * 0.5 * 1.003),
                        );
                    }
                });
                b += 1;
            }
        }
    }

    // ---- SHIMMER lane: octave-up choir, sections with fx `shimmer` ----
    let shimmer = score.param("shimmer", 0.09);
    if shimmer > 0.001 {
        for sec in &score.sections {
            if !sec.fx_on("shimmer") {
                continue;
            }
            if let Some((s0, s1)) = section_window(score, &sec.name) {
                let mut b = (s0 / bar).ceil() as usize;
                while (b as f32) * bar < s1 {
                    let t = b as f32 * bar;
                    let ramp = ((t - s0) / ((s1 - s0) * 0.6)).clamp(0.0, 1.0);
                    let tri = score.chord_at(t).triad();
                    ev(&mut lanes[L_SHIM], t, move |bd, _| {
                        for &f in tri.iter() {
                            render_into(
                                bd,
                                t,
                                bar,
                                shimmer * ramp,
                                -0.85,
                                choir_pick(padsw, f * 2.0),
                            );
                            render_into(
                                bd,
                                t,
                                bar,
                                shimmer * ramp,
                                0.85,
                                choir_pick(padsw, f * 2.0 * 1.004),
                            );
                        }
                    });
                    b += 1;
                }
            }
        }
    }

    // ---- STABS lane: donk / house organ / casio off-beats ----
    let hb = beat / 2.0;
    let stab_layer = |lanes: &mut [Vec<Event>; LANES],
                      tok: &str,
                      seed: u32,
                      def: f32,
                      mids_k: f32,
                      spread: f32,
                      voice: fn(f32) -> Box<dyn fundsp::prelude32::AudioUnit>,
                      dur: f32| {
        for sec in &score.sections {
            if !sec.fx_on(tok) {
                continue;
            }
            if let Some((s0, s1)) = section_window(score, &sec.name) {
                let mut t = (s0 / beat).ceil() * beat + hb;
                while t < s1 {
                    let m = score.levels(t).mids;
                    let gt = groove(t, beat, seed, 0.004, 0.0);
                    let amp = (score.param_at(t, tok, def) + mids_k * m) * vel(t, beat, seed);
                    let tri = score.chord_at(t).triad();
                    ev(&mut lanes[L_STABS], gt, move |bd, _| {
                        chord_spread(bd, gt, dur, amp, spread, tri, voice)
                    });
                    t += beat;
                }
            }
        }
    };
    stab_layer(&mut lanes, "donk", 0xD0, 0.055, 0.05, 0.55, donk, hb * 0.9);
    stab_layer(
        &mut lanes,
        "house",
        0x40,
        0.12,
        0.06,
        0.7,
        houseorg,
        hb * 0.95,
    );
    // casio uses a fixed amp formula (not param_at) + 0x4C seed — inline (doesn't fit stab_layer)
    let half = beat / 2.0;
    for sec in &score.sections {
        if !sec.fx_on("casio") {
            continue;
        }
        if let Some((s0, s1)) = section_window(score, &sec.name) {
            let mut t = (s0 / beat).ceil() * beat + half;
            while t < s1 {
                let m = score.levels(t).mids;
                let gt = groove(t, beat, 0x4C, 0.005, 0.0);
                let amp = (0.05 + 0.06 * m) * vel(t, beat, 0x4C);
                let tri = score.chord_at(t).triad();
                ev(&mut lanes[L_STABS], gt, move |bd, _| {
                    chord_spread(bd, gt, half * 0.95, amp, 0.5, tri, casio)
                });
                t += beat;
            }
        }
    }

    // ---- FX lane: risers / jets / impacts / snare-rolls (sub + atmosphere are finishers) ----
    if let Some(t) = section_time(score, "build")
        && score.fx_on("build", "riser")
    {
        let s = t - 2.0 * bar;
        ev(&mut lanes[L_FX], s, move |bd, _| {
            render_riser(bd, s, 2.0 * bar, 0.10, -0.25)
        });
    }
    if let Some(t) = section_time(score, "drop") {
        if score.fx_on("drop", "riser") {
            let s = t - 4.0 * bar;
            ev(&mut lanes[L_FX], s, move |bd, _| {
                render_riser(bd, s, 4.0 * bar, 0.26, 0.15)
            });
            let r = t - 2.0 * bar;
            ev(&mut lanes[L_FX], r, move |bd, _| {
                render_snare_roll(bd, r, 2.0 * bar, beat)
            });
        }
        if score.fx_on("drop", "jet") {
            let s = t - 3.0 * bar;
            ev(&mut lanes[L_FX], s, move |bd, _| {
                render_jet(bd, s, 3.0 * bar, 0.32)
            });
        }
        if score.fx_on("drop", "impact") {
            ev(&mut lanes[L_FX], t, move |bd, _| {
                render_impact(bd, t, 1.6, 0.62)
            });
        }
    }
    if let Some(t) = section_time(score, "breakdown")
        && score.fx_on("breakdown", "impact")
    {
        ev(&mut lanes[L_FX], t, move |bd, _| {
            render_impact(bd, t, 2.2, 0.38)
        });
    }
    if let Some(t) = section_time(score, "climax") {
        if score.fx_on("climax", "riser") {
            let s = t - 4.0 * bar;
            ev(&mut lanes[L_FX], s, move |bd, _| {
                render_riser(bd, s, 4.0 * bar, 0.34, -0.15)
            });
        }
        if score.fx_on("climax", "jet") {
            let s = t - 4.0 * bar;
            ev(&mut lanes[L_FX], s, move |bd, _| {
                render_jet(bd, s, 4.0 * bar, 0.5)
            });
        }
        if score.fx_on("climax", "impact") {
            ev(&mut lanes[L_FX], t, move |bd, _| {
                render_impact(bd, t, 2.0, 0.72)
            });
        }
    }
    if let Some(t0) = section_time(score, "outro") {
        let end = score.demo_len();
        if score.fx_on("outro", "riser") {
            let s = t0 - 3.0 * bar;
            ev(&mut lanes[L_FX], s, move |bd, _| {
                render_riser(bd, s, 3.0 * bar, 0.30, 0.0)
            });
            let r = t0 - 2.0 * bar;
            ev(&mut lanes[L_FX], r, move |bd, _| {
                render_snare_roll(bd, r, 2.0 * bar, beat)
            });
        }
        if score.fx_on("outro", "bang") {
            ev(&mut lanes[L_FX], t0, move |bd, _| {
                render_impact(bd, t0, 1.4, 0.5)
            });
            let build = (4.0 * bar).min(end - t0 - 0.1).max(0.5);
            let bs = end - build;
            ev(&mut lanes[L_FX], bs, move |bd, _| {
                render_snare_roll(bd, bs, build, beat)
            });
            ev(&mut lanes[L_FX], bs, move |bd, _| {
                render_riser(bd, bs, build, 0.42, 0.0)
            });
            let j = end - 2.6;
            ev(&mut lanes[L_FX], j, move |bd, _| {
                render_jet(bd, j, 2.0, 0.6)
            });
            let i1 = end - 1.9;
            ev(&mut lanes[L_FX], i1, move |bd, _| {
                render_impact(bd, i1, 2.2, 1.0)
            });
            let i2 = end - 0.45;
            ev(&mut lanes[L_FX], i2, move |bd, _| {
                render_impact(bd, i2, 1.0, 1.0)
            });
        }
    }

    lanes
}
