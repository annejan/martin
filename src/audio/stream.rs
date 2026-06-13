//! The **streaming synth engine**: renders the score in time-ordered segments so live playback can
//! start ~1 s after launch instead of waiting for the whole track. Each segment renders only the
//! events due in its window plus the resumable finishers (sub/atmosphere, the two ping-pong delays,
//! reverb, master), with all effect state carried across boundaries — so segment size provably does
//! not change the output (`segmenting_is_deterministic`), and the result matches the batch render
//! within float-summation-order noise (`stream_matches_batch`).
//!
//! This is a SEPARATE path from `synth_track` (the proven batch render that recordings + the bundle
//! use). It powers live windowed playback only, so a bug here can never affect a recording. The
//! scheduling lives in `render::collect_events` (a mirror of the batch pass functions); the two are
//! guarded against drift by `stream_matches_batch`. Once the stream is trusted by ear, batch can be
//! switched onto `produce` too and the duplication removed.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use super::{SAMPLE_RATE, section_time, section_window, set_oversampling, sub_freq};
use crate::score::{Inst, Score};

/// One scheduled note/hit/accent. `t` is the RAW musical start used only to bucket it into a
/// segment; `render` writes the event's FULL duration into `(target, kickbuf)` (events may spill
/// into later segments — fine, a sample is finalized only once every event that can touch it has
/// fired, which is exactly when its own segment is processed). Closures are `'static` (they capture
/// resolved `f32`s + call free voice fns), so collecting them needs no borrow of the score.
/// An event's render closure: writes its voice(s) into `(target_buffer, kickbuf)`.
pub(crate) type RenderFn = Box<dyn FnOnce(&mut [f32], &mut [f32]) + Send>;

pub(crate) struct Event {
    pub t: f32,
    pub render: RenderFn,
}

/// Fixed lane order — also the per-sample summation order into the bed, so it is part of the sound.
pub(crate) const LANES: usize = 10;
pub(crate) const L_DRUMS: usize = 0; // kick → kickbuf; bass-body / intro-perc / snare / hat / stab → bed
pub(crate) const L_LEAD: usize = 1; // intro bassline + the lead hook (+ sheens) → bed
pub(crate) const L_ECHO: usize = 2; // lead echo source → echo buf (ping-ponged, wet-only added)
pub(crate) const L_ARP: usize = 3; // arp counter-line → arp buf (ping-ponged whole)
pub(crate) const L_BASS: usize = 4; // articulated bassline → bed
pub(crate) const L_PAD: usize = 5; // sustained pad → bed
pub(crate) const L_WALL: usize = 6; // supersaw + choir wall → bed
pub(crate) const L_SHIM: usize = 7; // octave-up shimmer → bed
pub(crate) const L_STABS: usize = 8; // donk / house organ / casio off-beats → bed
pub(crate) const L_FX: usize = 9; // risers / jets / impacts / snare rolls → bed

// --------------------------------------------------------------- resumable effect finishers ----

/// Ping-pong delay (port of the old whole-buffer `render_pingpong`; state carried across ranges).
struct PingPong {
    line: Vec<f32>,
    w: usize,
    alt: u32,
    wet: f32,
    feedback: f32,
}

impl PingPong {
    fn new(delay_s: f32, feedback: f32, wet: f32) -> Self {
        let d = ((delay_s * SAMPLE_RATE as f32) as usize).max(2);
        PingPong {
            line: vec![0f32; d * 2],
            w: 0,
            alt: 0,
            wet,
            feedback,
        }
    }
    /// Process frames `[f0, f1)` in place — read the dry, add the delayed (wet), advance the line.
    fn process(&mut self, buf: &mut [f32], f0: usize, f1: usize) {
        let d = self.line.len() / 2;
        for i in f0..f1 {
            let mono = buf[2 * i] + buf[2 * i + 1];
            let dl = self.line[2 * self.w];
            let dr = self.line[2 * self.w + 1];
            buf[2 * i] += dl * self.wet;
            buf[2 * i + 1] += dr * self.wet;
            let fb = mono * self.feedback * 0.5;
            if self.alt & 1 == 0 {
                self.line[2 * self.w] = fb * 0.45;
                self.line[2 * self.w + 1] = fb;
            } else {
                self.line[2 * self.w] = fb;
                self.line[2 * self.w + 1] = fb * 0.45;
            }
            self.alt = self.alt.wrapping_add(1);
            self.w = (self.w + 1) % d;
        }
    }
}

struct Comb {
    line: Vec<f32>, // ring, length = delay
    w: usize,
    lp: f32,
}

struct AllPass {
    buf: Vec<f32>,
    w: usize,
    g: f32,
}

impl AllPass {
    fn step(&mut self, x: f32) -> f32 {
        let dl = self.buf[self.w]; // output `len` samples ago
        let y = -self.g * x + dl;
        self.buf[self.w] = x + self.g * y;
        self.w = (self.w + 1) % self.buf.len();
        y
    }
}

struct Tank {
    combs: Vec<Comb>,
    aps: [AllPass; 3],
}

/// Spread reverb (port of `reverb_send`): hp'd mono → 22 ms pre-delay → per-tank 6 feedback combs +
/// 3 series all-passes → darkened wet L/R. All state carried; processes ranges into `wet`.
struct Reverb {
    hp: f32,
    pre: Vec<f32>,
    pre_w: usize,
    pre_filled: usize,
    tanks: [Tank; 2],
    dark: [f32; 2],
    dark_a: f32,
    hp_a: f32,
}

impl Reverb {
    fn new(sr: f32) -> Self {
        let pre = (0.022 * sr) as usize;
        let tank = |delays: &[usize], ap_d: [f32; 3]| Tank {
            combs: delays
                .iter()
                .map(|&d| Comb {
                    line: vec![0f32; d],
                    w: 0,
                    lp: 0.0,
                })
                .collect(),
            aps: ap_d.map(|d| AllPass {
                buf: vec![0f32; ((d * sr) as usize).max(1)],
                w: 0,
                g: 0.7,
            }),
        };
        Reverb {
            hp: 0.0,
            pre: vec![0f32; pre.max(1)],
            pre_w: 0,
            pre_filled: 0,
            tanks: [
                tank(
                    &[1117, 1188, 1277, 1356, 1422, 1491],
                    [0.0051, 0.0167, 0.0097],
                ),
                tank(
                    &[1129, 1213, 1291, 1373, 1447, 1499],
                    [0.0047, 0.0153, 0.0089],
                ),
            ],
            dark: [0.0; 2],
            dark_a: 1.0 - (-std::f32::consts::TAU * 6500.0 / sr).exp(),
            hp_a: 1.0 - (-std::f32::consts::TAU * 300.0 / sr).exp(),
        }
    }

    fn process(&mut self, bed: &[f32], wet: &mut [f32], f0: usize, f1: usize) {
        let damp = 0.25_f32;
        let pre_len = self.pre.len();
        for i in f0..f1 {
            let m = 0.5 * (bed[2 * i] + bed[2 * i + 1]);
            self.hp += self.hp_a * (m - self.hp);
            let mono = m - self.hp;
            let src = if self.pre_filled >= pre_len {
                self.pre[self.pre_w]
            } else {
                0.0
            };
            self.pre[self.pre_w] = mono;
            self.pre_w = (self.pre_w + 1) % pre_len;
            self.pre_filled = self.pre_filled.saturating_add(1);
            for (c, tank) in self.tanks.iter_mut().enumerate() {
                let mut w = 0f32;
                for comb in tank.combs.iter_mut() {
                    let d = comb.line.len();
                    let fb_in = comb.line[comb.w];
                    comb.lp += damp * (fb_in - comb.lp);
                    comb.line[comb.w] = src + 0.88 * comb.lp;
                    comb.w = (comb.w + 1) % d;
                    w += 0.88 * comb.lp;
                }
                w *= 0.5;
                for ap in tank.aps.iter_mut() {
                    w = ap.step(w);
                }
                self.dark[c] += self.dark_a * (w - self.dark[c]);
                wet[2 * i + c] = self.dark[c];
            }
        }
    }
}

/// Continuous sub-bass oscillator + the atmosphere noise/crackle bed (ported from `render_fx`).
struct SubAtmo {
    phase: f32,
    sub_hz: f32,
    atmo_lp: f32,
    atmo_hp: f32,
    atmo_start: usize,
    atmo_a: f32,
    atmo_ah: f32,
}

impl SubAtmo {
    fn process(&mut self, bed: &mut [f32], score: &Score, f0: usize, f1: usize, sr: f32, amt: f32) {
        use std::f32::consts::TAU;
        let dt = 1.0 / sr;
        let glide = 1.0 - (-dt / 0.045).exp();
        let fade = (1.5 * sr) as usize;
        for i in f0..f1 {
            let t = i as f32 * dt;
            self.sub_hz += (sub_freq(score.chord_at(t).root) - self.sub_hz) * glide;
            self.phase = (self.phase + TAU * self.sub_hz * dt) % TAU;
            let s = (self.phase.sin()
                + (self.phase * 2.0).sin() * 0.42
                + (self.phase * 3.0).sin() * 0.16)
                * (0.14 + score.param("sub", 0.46) * score.levels(t).sub_bass);
            bed[2 * i] += s;
            bed[2 * i + 1] += s;
            if amt > 0.0 && i >= self.atmo_start {
                let g = ((i - self.atmo_start) as f32 / fade as f32).min(1.0);
                let n = super::pseudo_noise(i * 2 + 7);
                self.atmo_lp += self.atmo_a * (n - self.atmo_lp);
                self.atmo_hp += self.atmo_ah * (self.atmo_lp - self.atmo_hp);
                let floor = (self.atmo_lp - self.atmo_hp) * 0.008;
                let crackle = if super::pseudo_noise(i * 3 + 1) > 0.9996 {
                    super::pseudo_noise(i * 7) * 0.03
                } else {
                    0.0
                };
                let v = (floor + crackle) * g * amt;
                bed[2 * i] += v;
                bed[2 * i + 1] += v * 0.92;
            }
        }
    }
}

/// Haas widen + the 2-band master chain (port of `master`'s per-sample tail), state carried.
struct MasterChain {
    haas_buf: Vec<f32>,
    haas_filled: usize,
    hp: f32,
    bp: f32,
    hp_a: f32,
    lp_a: f32,
    lp: [f32; 2],
    air_lp: [f32; 2],
    gr: f32,
    glue: f32,
    g_env: f32,
    split_k: f32,
    atk: f32,
    rel: f32,
    g_atk: f32,
    g_rel: f32,
    reverb_amt: f32,
    widen: f32,
    makeup: f32,
    ceiling: f32,
}

impl MasterChain {
    fn new(score: &Score, sr: f32) -> Self {
        use std::f32::consts::TAU;
        let dt = 1.0 / sr;
        MasterChain {
            haas_buf: vec![0f32; (0.012 * sr) as usize],
            haas_filled: 0,
            hp: 0.0,
            bp: 0.0,
            hp_a: 1.0 - (-TAU * 600.0 / sr).exp(),
            lp_a: 1.0 - (-TAU * 6000.0 / sr).exp(),
            lp: [0.0; 2],
            air_lp: [0.0; 2],
            gr: 1.0,
            glue: 1.0,
            g_env: 0.0,
            split_k: 1.0 - (-TAU * 160.0 * dt).exp(),
            atk: 1.0 - (-1.0 / (0.0006 * sr)).exp(),
            rel: 1.0 - (-1.0 / (0.12 * sr)).exp(),
            g_atk: 1.0 - (-1.0 / (0.010 * sr)).exp(),
            g_rel: 1.0 - (-1.0 / (0.18 * sr)).exp(),
            reverb_amt: score.param("reverb", 0.35),
            widen: score.param("widen", 1.55),
            makeup: score.param("makeup", 1.18),
            ceiling: score.param("ceiling", 0.93),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn process(
        &mut self,
        kickbuf: &[f32],
        bed: &mut [f32],
        wet: &[f32],
        duck: &[f32],
        rv_env: &[f32],
        out: &mut [f32],
        score: &Score,
        f0: usize,
        f1: usize,
        sr: f32,
    ) {
        let dt = 1.0 / sr;
        let demo = score.demo_len();
        let haas_d = self.haas_buf.len();
        for i in f0..f1 {
            let m = 0.5 * (bed[2 * i] + bed[2 * i + 1]);
            self.hp += self.hp_a * (m - self.hp);
            let h = m - self.hp;
            self.bp += self.lp_a * (h - self.bp);
            let w = i % haas_d;
            let delayed = if self.haas_filled >= haas_d {
                self.haas_buf[w]
            } else {
                0.0
            };
            self.haas_buf[w] = self.bp;
            self.haas_filled = self.haas_filled.saturating_add(1);
            bed[2 * i + 1] += delayed * 0.25;

            let t = i as f32 * dt;
            let fade_in = (t / 1.5).clamp(0.0, 1.0);
            let fade_out = ((demo - t) / 0.025).clamp(0.0, 1.0);
            let g = fade_in * fade_out * score.gain_at(t);
            let mut lo = [0.0f32; 2];
            let mut hi = [0.0f32; 2];
            for c in 0..2 {
                let x = kickbuf[2 * i + c]
                    + bed[2 * i + c] * duck[i]
                    + wet[2 * i + c] * self.reverb_amt * rv_env[i];
                self.lp[c] += self.split_k * (x - self.lp[c]);
                lo[c] = self.lp[c];
                hi[c] = x - self.lp[c];
            }
            let mm = 0.5 * (hi[0] + hi[1]);
            let ss = 0.5 * (hi[0] - hi[1]) * self.widen;
            hi[0] = mm + ss;
            hi[1] = mm - ss;
            let mut pre = [0.0f32; 2];
            for c in 0..2 {
                self.air_lp[c] += 0.5 * (hi[c] - self.air_lp[c]);
                let air = hi[c] - self.air_lp[c];
                let hi_x = hi[c] + (air * 1.5).tanh() * 0.3;
                let hi_s = (hi_x * 1.25).tanh();
                pre[c] = (lo[c] + hi_s) * g;
            }
            let det = pre[0].abs().max(pre[1].abs());
            self.g_env += (det - self.g_env)
                * if det > self.g_env {
                    self.g_atk
                } else {
                    self.g_rel
                };
            let thr = 0.5;
            let gtarget = if self.g_env > thr {
                (thr / self.g_env).sqrt()
            } else {
                1.0
            };
            self.glue += (gtarget - self.glue)
                * if gtarget < self.glue {
                    self.g_atk
                } else {
                    self.g_rel
                };
            pre[0] *= self.glue * self.makeup;
            pre[1] *= self.glue * self.makeup;
            let peak = pre[0].abs().max(pre[1].abs());
            let target = if peak > self.ceiling {
                self.ceiling / peak
            } else {
                1.0
            };
            if target < self.gr {
                self.gr += (target - self.gr) * self.atk;
            } else {
                self.gr += (1.0 - self.gr) * self.rel;
            }
            for c in 0..2 {
                out[2 * i + c] = (pre[c] * self.gr).clamp(-1.0, 1.0);
            }
        }
    }
}

// -------------------------------------------------------------------------- the producer ----

/// Segment size — small ⇒ playback starts fast; provably irrelevant to the output.
const SEG_SECS: f32 = 0.5;

/// Render the whole score in time order, calling `sink(chunk)` with each finalized stereo chunk.
/// Single-threaded + deterministic. Updates the global produced-frames counter for the loader.
pub(crate) fn produce(score: &Score, mut sink: impl FnMut(&[f32])) {
    let oversample = score.param("oversample", 0.0) > 0.5;
    set_oversampling(oversample);
    let sr = SAMPLE_RATE as f32;
    let total = (score.demo_len() * sr).ceil() as usize;
    let stereo = total * 2;
    super::progress_reset();

    // sidechain duck + reverb-depth automation (cheap precompute, no audio data needed)
    let kicks = score.hits(Inst::Kick);
    let mut duck = vec![1.0f32; total];
    let (depth, tau) = (score.param("sidechain", 0.78), 0.085f32);
    for &kt in &kicks {
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
        let dt = 1.0 / sr;
        let sm = 1.0 - (-dt / 0.3).exp();
        let mut e = target.first().copied().unwrap_or(1.0);
        for i in 0..total {
            e += (target[i] - e) * sm;
            rv_env[i] = 1.0 + reverbauto * (e - 1.0);
        }
    }

    // time-ordered events per lane
    let mut lanes = super::render::collect_events(score);
    for lane in lanes.iter_mut() {
        lane.sort_by(|a, b| a.t.partial_cmp(&b.t).unwrap_or(std::cmp::Ordering::Equal));
    }
    let mut cursors = [0usize; LANES];

    let mut kickbuf = vec![0f32; stereo];
    let mut bed = vec![0f32; stereo];
    let mut echo_buf = vec![0f32; stereo];
    let mut arp_buf = vec![0f32; stereo];
    let mut wet = vec![0f32; stereo];
    let mut out = vec![0f32; stereo];

    let beat = score.beat();
    let mut pp_echo = PingPong::new(beat * 0.75, 0.34, 0.55);
    let mut pp_arp = PingPong::new(beat / 2.0, 0.35, 0.30);
    let mut reverb = Reverb::new(sr);
    let mut sub_atmo = SubAtmo {
        phase: 0.0,
        sub_hz: sub_freq(score.chord_at(0.0).root),
        atmo_lp: 0.0,
        atmo_hp: 0.0,
        atmo_start: (section_time(score, "build").unwrap_or(0.0) * sr) as usize,
        atmo_a: 1.0 - (-std::f32::consts::TAU * 2000.0 / sr).exp(),
        atmo_ah: 1.0 - (-std::f32::consts::TAU * 350.0 / sr).exp(),
    };
    let atmo_amt = score.param("atmosphere", 1.0);
    let mut master = MasterChain::new(score, sr);

    let seg_frames = (SEG_SECS * sr) as usize;
    let mut f0 = 0usize;
    while f0 < total {
        let f1 = (f0 + seg_frames).min(total);
        let seg_end_t = f1 as f32 / sr;
        // 1. fire due events, fixed lane order (Echo/Arp into their own buffers)
        for (li, lane) in lanes.iter_mut().enumerate() {
            let target: &mut [f32] = match li {
                L_ECHO => &mut echo_buf,
                L_ARP => &mut arp_buf,
                _ => &mut bed,
            };
            while cursors[li] < lane.len() && lane[cursors[li]].t < seg_end_t {
                let render = std::mem::replace(
                    &mut lane[cursors[li]].render,
                    Box::new(|_: &mut [f32], _: &mut [f32]| {}),
                );
                render(target, &mut kickbuf);
                cursors[li] += 1;
            }
        }
        // 2. resumable finishers over [f0, f1)
        sub_atmo.process(&mut bed, score, f0, f1, sr, atmo_amt);
        let dry = echo_buf[2 * f0..2 * f1].to_vec(); // capture before pingpong → wet = after - dry
        pp_echo.process(&mut echo_buf, f0, f1);
        for (k, i) in (f0..f1).enumerate() {
            bed[2 * i] += echo_buf[2 * i] - dry[2 * k];
            bed[2 * i + 1] += echo_buf[2 * i + 1] - dry[2 * k + 1];
        }
        pp_arp.process(&mut arp_buf, f0, f1);
        for i in f0..f1 {
            bed[2 * i] += arp_buf[2 * i];
            bed[2 * i + 1] += arp_buf[2 * i + 1];
        }
        reverb.process(&bed, &mut wet, f0, f1);
        master.process(
            &kickbuf, &mut bed, &wet, &duck, &rv_env, &mut out, score, f0, f1, sr,
        );
        // 3. hand off the finalized frames
        sink(&out[2 * f0..2 * f1]);
        super::progress_advance(f1 - f0);
        f0 = f1;
    }
}

// ------------------------------------------------------------------------- the live stream ----

/// The growing track the producer fills and the audio decoder reads. Finalized segments are
/// appended under a mutex; the decoder caches the current segment `Arc` so the audio thread only
/// locks at segment boundaries (~every 0.5 s).
pub(crate) struct StreamBuf {
    segments: Mutex<Vec<Arc<Vec<f32>>>>,
    finalized: AtomicUsize, // stereo frames finalized so far
    total: usize,
}

impl StreamBuf {
    pub(crate) fn new(total_frames: usize) -> Self {
        StreamBuf {
            segments: Mutex::new(Vec::new()),
            finalized: AtomicUsize::new(0),
            total: total_frames,
        }
    }
    pub(crate) fn push(&self, chunk: &[f32]) {
        self.segments.lock().unwrap().push(Arc::new(chunk.to_vec()));
        self.finalized.fetch_add(chunk.len() / 2, Ordering::Release);
    }
    pub(crate) fn finalized_frames(&self) -> usize {
        self.finalized.load(Ordering::Acquire)
    }
    pub(crate) fn total_frames(&self) -> usize {
        self.total
    }
    fn segment(&self, idx: usize) -> Option<Arc<Vec<f32>>> {
        self.segments.lock().unwrap().get(idx).cloned()
    }
}

/// Reads the growing stream as interleaved stereo `f32` at `SAMPLE_RATE`. If playback ever outruns
/// the producer (≥7× realtime headroom — practically unreachable) it pads silence rather than ending.
pub(crate) struct StreamDecoder {
    buf: Arc<StreamBuf>,
    seg: Option<Arc<Vec<f32>>>,
    seg_idx: usize,
    pos: usize,
}

impl StreamDecoder {
    pub(crate) fn new(buf: Arc<StreamBuf>) -> Self {
        StreamDecoder {
            buf,
            seg: None,
            seg_idx: 0,
            pos: 0,
        }
    }
}

impl Iterator for StreamDecoder {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        loop {
            if let Some(seg) = &self.seg {
                if self.pos < seg.len() {
                    let v = seg[self.pos];
                    self.pos += 1;
                    return Some(v);
                }
                self.seg = None;
                self.seg_idx += 1;
                self.pos = 0;
                continue;
            }
            match self.buf.segment(self.seg_idx) {
                Some(seg) => self.seg = Some(seg),
                None => {
                    if self.buf.finalized_frames() >= self.buf.total_frames() {
                        return None; // past the end → done
                    }
                    return Some(0.0); // underrun → silence (keeps the sink alive)
                }
            }
        }
    }
}

/// `SAMPLE_RATE` exposed for the bevy-audio glue in `music.rs`.
pub(crate) const STREAM_SR: u32 = SAMPLE_RATE;

// rodio `Source` (re-exported by bevy_audio) so a `Decodable` asset can wrap this decoder. The
// stream is unbounded-until-the-track-ends; `total_duration` reports the full length so the sink
// knows when to stop.
impl bevy::audio::Source for StreamDecoder {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> u16 {
        2
    }
    fn sample_rate(&self) -> u32 {
        STREAM_SR
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_secs_f64(
            self.buf.total_frames() as f64 / STREAM_SR as f64,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::score::Score;

    fn tiny_score() -> Score {
        Score::from_str(
            "bpm 120\nchords C F\nsection intro 2 2\nsection drop 2 2\n\
             intro.kick p0: x... .... x... ....\ndrop.kick p0: x.x. x.x. x.x. x.x.\n\
             drop.snare p0: .... x... .... x...\ngain intro 0.5 drop 1.0\n",
        )
        .expect("tiny score parses")
    }

    fn render_stream(score: &Score) -> Vec<f32> {
        let mut out = Vec::new();
        produce(score, |c| out.extend_from_slice(c));
        out
    }

    /// THE streaming guarantee #1: deterministic across runs (same order, same thread).
    #[test]
    fn segmenting_is_deterministic() {
        let score = tiny_score();
        let a = render_stream(&score);
        let b = render_stream(&score);
        assert_eq!(a.len(), b.len());
        assert!(
            a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()),
            "stream must be byte-deterministic"
        );
    }

    /// THE streaming guarantee #2: the stream matches the batch render within float-order noise —
    /// same DSP, only the per-sample summation order differs (segmented + lane order vs the batch's
    /// parallel passes). A gross port bug (silence, wrong length, distortion) blows past the bound.
    /// The decoder reads pushed segments in order, pads silence on underrun (producer behind), and
    /// ends once the full track is finalized.
    #[test]
    fn decoder_reads_then_pads_then_ends() {
        let buf = Arc::new(StreamBuf::new(4)); // 4 stereo frames = 8 samples total
        buf.push(&[0.1, 0.2, 0.3, 0.4]); // 2 frames
        let mut d = StreamDecoder::new(buf.clone());
        assert_eq!(
            [d.next(), d.next(), d.next(), d.next()],
            [Some(0.1), Some(0.2), Some(0.3), Some(0.4)]
        );
        assert_eq!(d.next(), Some(0.0)); // underrun (not yet total) → silence, not end
        buf.push(&[0.5, 0.6, 0.7, 0.8]); // rest arrives → total reached
        assert_eq!([d.next(), d.next()], [Some(0.5), Some(0.6)]);
        assert_eq!([d.next(), d.next()], [Some(0.7), Some(0.8)]);
        assert_eq!(d.next(), None); // finalized == total → done
    }

    #[test]
    fn stream_matches_batch() {
        let score = tiny_score();
        let stream = render_stream(&score);
        let batch = crate::audio::synth_track(&score);
        let batch = &*batch.samples; // private field — visible to this descendant module
        assert_eq!(
            stream.len(),
            batch.len(),
            "length must match the batch render"
        );
        let n = stream.len();
        let mut max = 0f32;
        let mut sum = 0f64;
        for (a, b) in stream.iter().zip(batch.iter()) {
            let d = (a - b).abs();
            max = max.max(d);
            sum += d as f64;
        }
        let mean = (sum / n as f64) as f32;
        assert!(
            max < 0.06 && mean < 0.004,
            "stream diverged from batch: max {max}, mean {mean} (expected float-order noise only)"
        );
    }
}
