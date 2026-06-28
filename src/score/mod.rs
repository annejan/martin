//! Music score — the *composition*, data-driven. Ported from Cinder's (Kristian Vlaardingerbroek,
//! deFEEST) `term-demo` (MIT, Outline 2026): the BPM→beat→bar grid, the section timeline
//! (intro→build→drop→breakdown→climax→outro), the drum patterns and the per-section dynamics that
//! the synth (the `audio` module) and the visual `@@anchor`s both read.
//!
//! The music lives in a **text file**, not in code: `assets/score.txt` (a tracker-DSL score) is
//! loaded by default — edit it, no recompile — and `include_str!`'d as the embedded fallback for a
//! bundled binary (so the notes/patterns/chords are not duplicated in Rust). `MARTIN_SCORE=<file>`
//! overrides it; `MARTIN_SCORE_DUMP=<file>` writes a copy. The *instrument* (how a kick/stab
//! sounds) stays in the `audio` module. 16 steps per bar (16th notes).
//!
//! This module is split by concern: `types` (the data), `parse` (text → `Score`), `dump`
//! (`Score` → text), `validate` (the structural lint), and the timeline maths on `Score` below.

mod dump;
mod parse;
mod types;
mod validate;

pub use types::{Chord, Inst, Levels, NoteLane, Ramp, Section, TempoPoint};

const SLOTS_PER_BAR: i64 = 16;
const BEATS_PER_BAR: f32 = 4.0;

/// Hard ceiling on a score's playable length (seconds) — the streaming synth allocates ~16 buffers of
/// `demo_len()*sr`, so an absurdly-long score (a `section … x 10000`, hundreds of section lines) must
/// not be able to demand tens of GB. Real tracks are ~90–170 s; 10 min is far past any demo.
/// [`Score::demo_len`] clamps to this; [`validate`] warns (fatal under strict) before you get here.
pub(crate) const MAX_DEMO_SECS: f32 = 600.0;

/// Aggregate bar ceiling for the `validate` heads-up (the per-section guard is `parse::MAX_BARS`;
/// this catches the SUM across many sections). ~real tracks are tens of bars.
pub(crate) const MAX_TOTAL_BARS: u32 = 10_000;

/// The editable default score, loaded from disk when present (so editing it needs no recompile).
/// The same file is `include_str!`'d as the embedded fallback — the music lives here, not in code.
const DEFAULT_SCORE: &str = "assets/score.txt";

/// Crossfade window (seconds) smoothing per-section dynamics steps at boundaries — long enough to
/// kill the click, short enough not to smear the musical transition.
pub const SECTION_FADE: f32 = 0.12;

/// A whole score: tempo + an ordered list of sections (which carry their own patterns + dynamics).
#[derive(Clone)]
pub struct Score {
    pub bpm: f32,               // base / bar-0 tempo (the `bpm N` line)
    pub tempo: Vec<TempoPoint>, // tempo changes (sorted by bar); EMPTY = constant tempo (= today)
    pub chords: Vec<Chord>,     // per-bar chord progression (cycles); drives bass + stab
    pub sections: Vec<Section>,
    total_bars: u32,
    /// Cumulative seconds at the START of each bar, length `total_bars + 1` — the prefix sum that makes
    /// the slot↔seconds map piecewise-per-bar (a bar's duration = its `grid`/4 beats at its tempo).
    bar_secs: Vec<f32>,
    /// Cumulative SLOT count at the start of each bar, length `total_bars + 1` — maps slot↔bar when
    /// bars have different `grid`s. With all-16 grids it is exactly `bar_slot0[b] == b * 16`.
    bar_slot0: Vec<i64>,
    /// `tempo` empty AND every section grid == 16 — then the funnels take the exact old single-multiply
    /// fast path and the whole timeline is byte-for-byte identical to a pre-tempo, pre-grid score.
    uniform: bool,
    /// Free-form mix/fx knobs from `set <key>=<value>` lines — the synth reads these (with built-in
    /// defaults) so the SOUND can be tuned by editing the score file (no recompile), not the engine.
    params: std::collections::HashMap<String, f32>,
}

impl Score {
    /// Lay out the sections (cumulative `start_bar`, total length) — the single place section
    /// timing is derived, so the file and the built-in agree.
    fn new(
        bpm: f32,
        tempo: Vec<TempoPoint>,
        chords: Vec<Chord>,
        mut sections: Vec<Section>,
    ) -> Self {
        let mut bar = 0;
        for s in &mut sections {
            s.start_bar = bar;
            bar += s.bars;
        }
        let total_bars = bar;
        // Per-bar grid (slots): the section a bar falls in sets it (16 = 4/4 sixteenths, the default).
        let mut bar_grid = vec![16usize; total_bars as usize];
        for s in &sections {
            for b in s.start_bar..s.start_bar + s.bars {
                if let Some(g) = bar_grid.get_mut(b as usize) {
                    *g = s.grid.max(1);
                }
            }
        }
        // Prefix-sum BOTH the per-bar seconds and the per-bar slot count. A bar's duration is its
        // grid/4 beats (a slot is always a 16th) at the latest tempo point with `point.bar <= b`
        // (default `bpm`). All-16 grids + empty tempo ⇒ `bar_secs[b] == b*4*(60/bpm)` and
        // `bar_slot0[b] == b*16` ⇒ byte-identical to before.
        let mut bar_secs = Vec::with_capacity(total_bars as usize + 1);
        let mut bar_slot0 = Vec::with_capacity(total_bars as usize + 1);
        let mut acc = 0.0f32;
        let mut slot_acc = 0i64;
        let mut cur_bpm = bpm;
        let mut ti = 0;
        for b in 0..=total_bars {
            bar_secs.push(acc);
            bar_slot0.push(slot_acc);
            if b == total_bars {
                break;
            }
            while ti < tempo.len() && tempo[ti].bar <= b {
                cur_bpm = tempo[ti].bpm;
                ti += 1;
            }
            let g = bar_grid[b as usize];
            acc += (g as f32 / 4.0) * (60.0 / cur_bpm); // grid/4 beats at the current tempo
            slot_acc += g as i64;
        }
        let uniform = tempo.is_empty() && bar_grid.iter().all(|&g| g == 16);
        // a score with no `chords` line still needs harmony — default to a single A-minor.
        let chords = if chords.is_empty() {
            vec![Chord {
                root: parse::note_freq("A3").unwrap(),
                minor: true,
            }]
        } else {
            chords
        };
        Self {
            bpm,
            tempo,
            chords,
            sections,
            total_bars,
            bar_secs,
            bar_slot0,
            uniform,
            params: std::collections::HashMap::new(),
        }
    }

    /// A mix/fx knob (`set <key>=<value>` in the score), or `default` if unset — the single hook the
    /// synth uses so its levels/sends live in the score file, tunable without recompiling the engine.
    pub fn param(&self, key: &str, default: f32) -> f32 {
        self.params.get(key).copied().unwrap_or(default)
    }

    /// A mix/fx knob honouring a per-section override: if the section active at `t` has a
    /// `<section>.set key=…` for `key`, return that; otherwise fall back to the global `param`. The
    /// synth uses this for the knobs it reads per-onset (so a section can be louder/quieter without a
    /// recompile and without touching the others — e.g. `drop.set house=0.18`).
    pub fn param_at(&self, t: f32, key: &str, default: f32) -> f32 {
        let s = &self.sections[self.section_index_at(t)];
        s.params
            .get(key)
            .copied()
            .unwrap_or_else(|| self.param(key, default))
    }

    /// Whether the section named `name` gets FX/layer `token` (see `Section::fx_on`). Unknown section
    /// → false. The synth gates its accents on this so a section's FX live in the score, not in code.
    pub fn fx_on(&self, name: &str, token: &str) -> bool {
        self.sections
            .iter()
            .find(|s| s.name == name)
            .is_some_and(|s| s.fx_on(token))
    }

    // --- grid ---------------------------------------------------------------------------------
    // `beat()`/`bar()` are the NOMINAL (bar-0 / base) lengths — used where one representative value is
    // wanted (fixed delay times, default travel durations). For position-correct timing under tempo
    // automation use `beat_at(t)` / `bar_len_at(t)` / `bar_start_secs(b)`, and convert slot↔seconds
    // ONLY through `slot_to_secs` / `secs_to_slot` (the two funnels). With an empty `tempo` all of these
    // collapse to the constant-tempo formulas, so existing scores render byte-identically.
    pub fn beat(&self) -> f32 {
        60.0 / self.bpm
    }
    pub fn bar(&self) -> f32 {
        BEATS_PER_BAR * self.beat()
    }
    // Every funnel below has a CONSTANT-TEMPO fast path (`tempo.is_empty()`) that reproduces the exact
    // old single-multiply formula — so a score with no `tempo` line is byte-for-byte identical (the
    // accumulated table would otherwise differ by a ULP and break the round-trip / anchor asserts).
    /// Nominal 16th-slot length (constant-tempo / bar-0).
    #[inline]
    fn slot_len(&self) -> f32 {
        self.beat() / 4.0
    }
    /// Slots in `bar` (its `grid`), from the cumulative table. All-16 grids ⇒ 16.
    #[inline]
    fn bar_grid(&self, bar: u32) -> i64 {
        let b = (bar as usize).min(self.total_bars.saturating_sub(1) as usize);
        self.bar_slot0[b + 1] - self.bar_slot0[b]
    }
    /// Total slots across the whole track (`sum of each bar's grid`). All-16 ⇒ `total_bars * 16`.
    #[inline]
    fn total_slots(&self) -> i64 {
        self.bar_slot0[self.total_bars as usize]
    }
    /// The bar whose `[bar_slot0[b], bar_slot0[b+1])` span contains absolute slot `slot`.
    fn bar_of_slot(&self, slot: i64) -> u32 {
        (match self.bar_slot0.binary_search(&slot) {
            Ok(b) => b.min(self.total_bars.saturating_sub(1) as usize),
            Err(0) => 0,
            Err(b) => (b - 1).min(self.total_bars.saturating_sub(1) as usize),
        }) as u32
    }
    /// Seconds spanned by one slot inside `bar` (the bar's length / its grid).
    #[inline]
    fn slot_len_of_bar(&self, bar: u32) -> f32 {
        let b = bar.min(self.total_bars.saturating_sub(1)) as usize;
        (self.bar_secs[b + 1] - self.bar_secs[b]) / self.bar_grid(bar) as f32
    }
    /// Seconds at the start of bar `bar` (clamped to the track).
    pub fn bar_start_secs(&self, bar: u32) -> f32 {
        let bar = (bar as usize).min(self.total_bars as usize);
        if self.uniform {
            bar as f32 * self.bar()
        } else {
            self.bar_secs[bar]
        }
    }
    /// Local beat length (s) at time `t` — the bar-at-`t`'s slot length × 4 (a beat = 4 slots).
    pub fn beat_at(&self, t: f32) -> f32 {
        if self.uniform {
            self.beat()
        } else {
            self.slot_len_of_bar(self.bar_idx_at(t)) * 4.0
        }
    }
    /// Local bar length (s) at time `t`.
    pub fn bar_len_at(&self, t: f32) -> f32 {
        if self.uniform {
            self.bar()
        } else {
            let b = (self.bar_idx_at(t) as usize).min(self.total_bars as usize);
            self.bar_secs[(b + 1).min(self.total_bars as usize)] - self.bar_secs[b]
        }
    }
    /// FORWARD funnel: an absolute slot index → seconds. Fractional `slot` lerps inside its bar.
    pub(crate) fn slot_to_secs(&self, slot: f32) -> f32 {
        if slot <= 0.0 {
            return 0.0;
        }
        if self.uniform {
            return slot * self.slot_len();
        }
        let bar = self.bar_of_slot(slot as i64);
        let rem = slot - self.bar_slot0[bar as usize] as f32;
        self.bar_start_secs(bar) + rem * self.slot_len_of_bar(bar)
    }
    /// INVERSE funnel: seconds → absolute slot index (floored). Keeps the forward epsilon nudge
    /// (per the local slot length) so an exact boundary time floors to the right slot, not one early.
    fn secs_to_slot(&self, t: f32) -> i64 {
        if self.uniform {
            let sl = self.slot_len();
            return ((t + sl * 1e-3) / sl).floor() as i64;
        }
        let bar = match self
            .bar_secs
            .binary_search_by(|x| x.partial_cmp(&t).unwrap_or(std::cmp::Ordering::Less))
        {
            Ok(b) => b.min(self.total_bars.saturating_sub(1) as usize),
            Err(0) => 0,
            Err(b) => (b - 1).min(self.total_bars.saturating_sub(1) as usize),
        } as u32;
        let sl = self.slot_len_of_bar(bar);
        let into = ((t - self.bar_start_secs(bar) + sl * 1e-3) / sl).floor() as i64;
        self.bar_slot0[bar as usize] + into.clamp(0, self.bar_grid(bar) - 1)
    }
    /// Total bar count (the playable timeline length in bars).
    pub fn total_bars(&self) -> u32 {
        self.total_bars
    }
    pub fn demo_len(&self) -> f32 {
        // CAP the reported length: the streaming synth sizes its up-front buffers from `demo_len()*sr`
        // (×~16 lanes), so a pathological score (`section … x 10000`, or hundreds of section lines)
        // would otherwise try to allocate tens of GB and OOM-abort instead of erroring cleanly. No real
        // track is anywhere near 10 min — clamp here so the allocation is bounded; `score::validate`
        // separately WARNS (fatal under strict) when a score is this long. (The recorder has its own
        // 1800 s clamp; this is the audio-buffer backstop.)
        let total = if self.uniform {
            self.total_bars as f32 * self.bar()
        } else {
            self.bar_secs[self.total_bars as usize]
        };
        total.min(MAX_DEMO_SECS)
    }

    fn abs_slot(&self, t: f32) -> i64 {
        self.secs_to_slot(t)
    }
    fn bar_idx_at(&self, t: f32) -> u32 {
        if self.uniform {
            return (self.abs_slot(t).max(0) / SLOTS_PER_BAR) as u32;
        }
        // the bar whose `[bar_secs[b], bar_secs[b+1])` span contains `t` (variable bar lengths).
        (match self
            .bar_secs
            .binary_search_by(|x| x.partial_cmp(&t).unwrap_or(std::cmp::Ordering::Less))
        {
            Ok(b) => b.min(self.total_bars.saturating_sub(1) as usize),
            Err(0) => 0,
            Err(b) => (b - 1).min(self.total_bars.saturating_sub(1) as usize),
        }) as u32
    }

    /// Structural lint of the tempo map (alongside `validate`'s section lints): a point past the
    /// score's length never fires, and a crawling tempo can balloon `demo_len` past the buffer clamp.
    pub(crate) fn tempo_warnings(&self) -> Vec<String> {
        let mut w = Vec::new();
        for p in &self.tempo {
            if p.bar >= self.total_bars {
                w.push(format!(
                    "tempo @bar:{} is past the score's {} bars — it never takes effect",
                    p.bar, self.total_bars
                ));
            }
            if p.bpm < 20.0 {
                w.push(format!(
                    "tempo @bar:{} = {} BPM is extremely slow — playback may hit the {:.0}s cap",
                    p.bar, p.bpm, MAX_DEMO_SECS
                ));
            }
        }
        w
    }

    // --- sections -----------------------------------------------------------------------------
    pub(crate) fn section_index_at(&self, t: f32) -> usize {
        let b = self.bar_idx_at(t);
        let mut idx = 0;
        for (i, s) in self.sections.iter().enumerate() {
            if b >= s.start_bar {
                idx = i;
            } else {
                break;
            }
        }
        idx
    }
    pub fn section_start_secs(&self, idx: usize) -> f32 {
        self.bar_start_secs(self.sections[idx].start_bar)
    }

    // --- patterns -----------------------------------------------------------------------------
    fn lane_hits(&self, inst: Inst, t: f32) -> &[bool] {
        let i = self.section_index_at(t);
        let s = &self.sections[i];
        let into = (self.bar_idx_at(t) as i64 - s.start_bar as i64).max(0) as u32;
        s.lane(inst).at(s.phase_at(into))
    }

    /// The position of absolute slot `s` within its bar (0..the bar's grid).
    #[inline]
    fn slot_in_bar(&self, s: i64) -> usize {
        (s - self.bar_slot0[self.bar_of_slot(s) as usize]) as usize
    }

    /// Every hit time (s) for `inst` across the whole track, in order — the synth builds a voice at
    /// each. Forward enumeration: walk every slot (per the bar's grid) and keep the ones that fire.
    pub fn hits(&self, inst: Inst) -> Vec<f32> {
        (0..self.total_slots())
            .filter_map(|s| {
                let t = self.slot_to_secs(s as f32);
                self.lane_hits(inst, t)
                    .get(self.slot_in_bar(s))
                    .copied()
                    .unwrap_or(false)
                    .then_some(t)
            })
            .collect()
    }

    // --- harmony + melody ---------------------------------------------------------------------
    /// The chord active at `t` (per-bar, cycling). A section with its own `chords:` line cycles
    /// through *that* progression (counted from the section start) — e.g. a G-minor verse under a
    /// G-major chorus; otherwise the global progression applies.
    pub fn chord_at(&self, t: f32) -> Chord {
        let bar = self.bar_idx_at(t) as usize;
        let s = &self.sections[self.section_index_at(t)];
        if !s.chords.is_empty() {
            return s.chords[(bar - s.start_bar as usize) % s.chords.len()];
        }
        self.chords[bar % self.chords.len()]
    }

    /// Chord root at `t` using a PRE-RESOLVED section index (avoids linear scan).
    pub(crate) fn chord_root_at(&self, t: f32, si: usize) -> f32 {
        let bar = self.bar_idx_at(t) as usize;
        let s = &self.sections[si];
        if !s.chords.is_empty() {
            s.chords[(bar - s.start_bar as usize) % s.chords.len()].root
        } else {
            self.chords[bar % self.chords.len()].root
        }
    }

    fn note_grid(&self, t: f32, pick: fn(&Section) -> &NoteLane) -> &[Option<f32>] {
        let i = self.section_index_at(t);
        let s = &self.sections[i];
        let into = (self.bar_idx_at(t) as i64 - s.start_bar as i64).max(0) as u32;
        // Use the SAME fill-bar sentinel the drum lanes use (phase_at → 255), not `into == bars-1`, so
        // the melodic fill and the drum fill always land on the same bar — they desync only when
        // phases+fill don't sum to bars (the already-linted typo). Identical for every clean score.
        let is_fill = s.phase_at(into) == 255;
        pick(s).bar(into, is_fill)
    }

    /// Every note of a note-lane as (time, freq) across the whole track — the synth builds a voice
    /// at each onset.
    /// Walk a note lane across the whole timeline → `(onset_secs, freq, hold_secs)` per note. A `-`/`_`
    /// **tie** slot (parsed as `NaN`) extends the previous note: `hold_secs` is the summed length of the
    /// tie slots after the onset, so the synth can hold the note instead of the fixed slot length. A
    /// note with no ties has `hold == 0.0` → byte-identical to before (every existing score uses `.`).
    fn note_line(&self, pick: fn(&Section) -> &NoteLane) -> Vec<(f32, f32, f32)> {
        let slots = self.total_slots();
        let secs = |s: i64| self.slot_to_secs(s as f32);
        let val = |s: i64| {
            self.note_grid(secs(s), pick)
                .get(self.slot_in_bar(s))
                .copied()
                .flatten()
        };
        let mut out = Vec::new();
        for s in 0..slots {
            // emit on a real onset only; a tie (NaN) is consumed by the note before it, a rest skipped.
            if let Some(f) = val(s)
                && !f.is_nan()
            {
                let onset_si = self.section_index_at(secs(s));
                let mut hold = 0i64;
                while s + 1 + hold < slots
                    && val(s + 1 + hold).is_some_and(|v| v.is_nan())
                    && self.section_index_at(secs(s + 1 + hold)) == onset_si
                {
                    hold += 1;
                }
                // hold = the tie span's seconds, SUMMED per slot (each spanned slot has its own length
                // once tempo varies) — not `count * one_slot`. Constant tempo → identical.
                out.push((secs(s), f, secs(s + 1 + hold) - secs(s + 1)));
            }
        }
        out
    }

    /// The `lead` (foreground melody) onsets — `(t, freq, hold)`; `hold` extends a tied note.
    pub fn lead_notes(&self) -> Vec<(f32, f32, f32)> {
        self.note_line(|s| &s.lead)
    }

    /// The `arp` (second melodic line) onsets — `(t, freq, hold)`.
    pub fn arp_notes(&self) -> Vec<(f32, f32, f32)> {
        self.note_line(|s| &s.arp)
    }

    /// The `bass` (articulated bassline) onsets — `(t, freq, hold)`; empty unless a `bass` lane exists.
    pub fn bass_notes(&self) -> Vec<(f32, f32, f32)> {
        self.note_line(|s| &s.bass)
    }

    // --- dynamics -----------------------------------------------------------------------------
    fn section_value<F: Fn(&Section) -> Ramp>(&self, t: f32, pick: &F) -> f32 {
        let i = self.section_index_at(t);
        self.section_value_at(i, t, pick)
    }

    /// Like `section_value` but uses a pre-resolved section index (avoids linear scan).
    pub(crate) fn section_value_at<F: Fn(&Section) -> Ramp>(
        &self,
        si: usize,
        t: f32,
        pick: &F,
    ) -> f32 {
        let s = &self.sections[si];
        let dur = (self.bar_start_secs(s.start_bar + s.bars) - self.bar_start_secs(s.start_bar))
            .max(1e-3);
        let p = ((t - self.section_start_secs(si)) / dur).clamp(0.0, 1.0);
        pick(s).at(p)
    }

    /// Crossfade a section value across its start boundary (`SECTION_FADE`) to remove the step.
    fn smooth<F: Fn(&Section) -> Ramp>(&self, t: f32, pick: F) -> f32 {
        let b = self.section_start_secs(self.section_index_at(t));
        let cur = self.section_value(t, &pick);
        if b > 0.0 && t - b < SECTION_FADE {
            let prev = self.section_value(b - 1e-3, &pick);
            prev + (cur - prev) * ((t - b) / SECTION_FADE)
        } else {
            cur
        }
    }

    /// Like `smooth` but uses a pre-resolved section index (avoids linear scan).
    pub(crate) fn smooth_at<F: Fn(&Section) -> Ramp>(&self, t: f32, si: usize, pick: F) -> f32 {
        let b = self.section_start_secs(si);
        let cur = self.section_value_at(si, t, &pick);
        if b > 0.0 && t - b < SECTION_FADE {
            let prev_si = self.section_index_at(b - 1e-3);
            let prev = self.section_value_at(prev_si, b - 1e-3, &pick);
            prev + (cur - prev) * ((t - b) / SECTION_FADE)
        } else {
            cur
        }
    }

    /// Master gain (per-section, crossfaded) the synth multiplies the mix by.
    pub fn gain_at(&self, t: f32) -> f32 {
        self.smooth(t, |s| s.gain)
    }

    /// Like `gain_at` but uses a pre-resolved section index (avoids linear scan).
    pub(crate) fn gain_at_cached(&self, t: f32, si: usize) -> f32 {
        self.smooth_at(t, si, |s| s.gain)
    }

    /// Sub-bass + mids levels: the per-section depth (crossfaded) under a slow LFO breath/swell.
    pub fn levels(&self, t: f32) -> Levels {
        use std::f32::consts::TAU;
        let breath = (t / (2.0 * self.beat()) * TAU).sin() * 0.5 + 0.5;
        let swell = (t / (8.0 * self.beat()) * TAU).sin() * 0.5 + 0.5;
        Levels {
            sub_bass: (breath * self.smooth(t, |s| s.sub)).clamp(0.0, 1.0),
            mids: (swell * self.smooth(t, |s| s.mids)).clamp(0.0, 1.0),
        }
    }

    /// Like `levels` but uses a pre-resolved section index (avoids linear scan).
    pub(crate) fn levels_cached(&self, t: f32, si: usize) -> Levels {
        use std::f32::consts::TAU;
        let breath = (t / (2.0 * self.beat()) * TAU).sin() * 0.5 + 0.5;
        let swell = (t / (8.0 * self.beat()) * TAU).sin() * 0.5 + 0.5;
        Levels {
            sub_bass: (breath * self.smooth_at(t, si, |s| s.sub)).clamp(0.0, 1.0),
            mids: (swell * self.smooth_at(t, si, |s| s.mids)).clamp(0.0, 1.0),
        }
    }

    // --- visual anchoring ---------------------------------------------------------------------
    /// Resolve an [`AnchorKind`] to an absolute **cue** time in seconds (the value behind the
    /// spelling). An unknown section name → `None`.
    pub fn cue(&self, kind: &AnchorKind) -> Option<f32> {
        match kind {
            AnchorKind::Start => Some(0.0),
            AnchorKind::Section(name) => self
                .sections
                .iter()
                .position(|x| x.name == *name)
                .map(|i| self.section_start_secs(i)),
            // bar/beat positions go through the funnel so an anchor lands at the right WALL-CLOCK time
            // under tempo automation (a beat = 4 slots, a bar = 16). Constant tempo → `b*bar()`/`b*beat()`.
            AnchorKind::Bar(b) => Some(self.slot_to_secs(b * SLOTS_PER_BAR as f32)),
            AnchorKind::Beat(b) => Some(self.slot_to_secs(b * 4.0)),
            AnchorKind::Seconds(s) => Some(*s),
            AnchorKind::Offset(base, sign, amount, unit) => self.cue(base).map(|b| {
                // constant tempo → the exact old formula (`base + amount * bar()/beat()`). Under tempo
                // automation a musical offset is measured in SLOTS at the base's location's tempo and
                // mapped back, so `@@drop+2bar` means 2 musical bars at the drop's tempo; raw-seconds
                // offsets are always wall-clock (never warped).
                if self.tempo.is_empty() {
                    return b + sign
                        * amount
                        * match unit {
                            OffsetUnit::Bar => self.bar(),
                            OffsetUnit::Beat => self.beat(),
                            OffsetUnit::Seconds => 1.0,
                        };
                }
                match unit {
                    OffsetUnit::Seconds => b + sign * amount,
                    OffsetUnit::Beat => {
                        self.slot_to_secs(self.secs_to_slot(b) as f32 + sign * amount * 4.0)
                    }
                    OffsetUnit::Bar => self.slot_to_secs(
                        self.secs_to_slot(b) as f32 + sign * amount * SLOTS_PER_BAR as f32,
                    ),
                }
            }),
        }
    }

    /// Resolve a `@@anchor` token straight to seconds — parse its [`AnchorKind`], then [`cue`] it
    /// (`@@drop`, `bar:16`, `beat8`, `start`, or raw seconds). Lets a shot lock to the music.
    ///
    /// [`cue`]: Score::cue
    pub fn anchor_seconds(&self, s: &str) -> Option<f32> {
        self.cue(&AnchorKind::parse(s)?)
    }
}

/// The parsed form of a `@@anchor` token — the **symbolic musical position** (the *spelling*). A
/// [`Score`] resolves it to a **cue** (absolute seconds, the *value*) via [`Score::cue`]. Formalises
/// what used to be ad-hoc string-sniffing inside `anchor_seconds` (DOMAIN.md §6).
#[derive(Debug, Clone, PartialEq)]
pub enum AnchorKind {
    /// `start` — time 0.
    Start,
    /// A named section (`drop`, `breakdown`, …) — resolves to that section's start. The name isn't
    /// validated here (no Score in hand); an unknown one resolves to `None`.
    Section(String),
    /// `bar<N>` / `bar:N` — N bars from the start.
    Bar(f32),
    /// `beat<N>` / `beat:N` — N beats from the start.
    Beat(f32),
    /// A raw number of seconds.
    Seconds(f32),
    /// A base anchor with an arithmetic suffix: `@@drop+2bar`, `@@beat:64-1beat`, `@@start+3s`. Stores
    /// the base, the sign (`+1`/`-1`), the amount, and the unit — a lead-in/lead-out relative to a cue.
    Offset(Box<AnchorKind>, f32, f32, OffsetUnit),
}

/// The unit of an offset suffix (`+2bar`, `-1beat`, `+3s`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum OffsetUnit {
    Bar,
    Beat,
    Seconds,
}

impl AnchorKind {
    /// Classify an anchor token. Order: `start` → `bar…`/`beat…` → a bare number → otherwise a
    /// section name. (Section names like `intro`/`drop` never collide with the numeric/prefix forms.)
    /// An optional arithmetic suffix `[+-]<amount>(bar|beat|b|s)` makes it relative — `drop+2bar`,
    /// `drop-1beat`, `bar:16-3s` — resolved against the base in [`Score::cue`].
    pub fn parse(s: &str) -> Option<Self> {
        let mut s: String = s.trim().to_ascii_lowercase();
        if s.is_empty() {
            return None;
        }
        // Strip the `@@` sigil that show-file anchors carry (`@@intro`, `@@bar:16`).
        if let Some(stripped) = s.strip_prefix("@@") {
            s = stripped.to_string();
        }
        // Split off an arithmetic suffix at the first `+`/`-` whose prefix is itself a valid base
        // (so a section literally named `foo-bar` stays one name, not `foo` minus `bar`).
        for (i, c) in s.char_indices() {
            if (c == '+' || c == '-') && i > 0 {
                let base = &s[..i];
                if Self::parse_base(base).is_some()
                    && let Some((amount, unit)) = Self::parse_offset(&s[i + 1..])
                {
                    let sign = if c == '+' { 1.0 } else { -1.0 };
                    return Some(Self::Offset(
                        Box::new(Self::parse_base(base)?),
                        sign,
                        amount,
                        unit,
                    ));
                }
            }
        }
        Self::parse_base(&s)
    }

    /// Parse just the base anchor (no suffix): `start` / `bar:N` / `beat:N` / a number / a section.
    fn parse_base(s: &str) -> Option<Self> {
        if s.is_empty() {
            return None;
        }
        if s == "start" {
            return Some(Self::Start);
        }
        if let Some(n) = s.strip_prefix("bar") {
            if let Ok(b) = n.trim_start_matches(':').parse::<f32>() {
                return Some(Self::Bar(b));
            }
        }
        if let Some(n) = s.strip_prefix("beat") {
            if let Ok(b) = n.trim_start_matches(':').parse::<f32>() {
                return Some(Self::Beat(b));
            }
        }
        if let Ok(secs) = s.parse::<f32>() {
            return Some(Self::Seconds(secs));
        }
        Some(Self::Section(s.to_string()))
    }

    /// Parse an offset amount+unit: `2bar` / `1beat` / `0.5b` / `3s`. (`b` = bar, bare number = beats.)
    fn parse_offset(s: &str) -> Option<(f32, OffsetUnit)> {
        let s = s.trim();
        if let Some(n) = s.strip_suffix("bar").or_else(|| s.strip_suffix('b')) {
            return n.parse::<f32>().ok().map(|a| (a, OffsetUnit::Bar));
        }
        if let Some(n) = s.strip_suffix("beat") {
            return n.parse::<f32>().ok().map(|a| (a, OffsetUnit::Beat));
        }
        if let Some(n) = s.strip_suffix('s') {
            return n.parse::<f32>().ok().map(|a| (a, OffsetUnit::Seconds));
        }
        s.parse::<f32>().ok().map(|a| (a, OffsetUnit::Beat)) // bare number → beats
    }
}

#[cfg(test)]
mod tests {
    use super::parse::note_freq;
    use super::validate::validate;
    use super::*;

    #[test]
    fn grid_is_consistent() {
        let s = Score::builtin();
        assert!(s.bpm > 0.0);
        assert!(s.beat() > 0.0);
        assert!((s.bar() - BEATS_PER_BAR * s.beat()).abs() < 1e-6);
        assert!(s.demo_len() > 0.0);
        assert!(!s.sections.is_empty());
    }

    #[test]
    fn section_starts_are_monotonic_and_first_is_zero() {
        let s = Score::builtin();
        assert_eq!(s.sections[0].start_bar, 0);
        let mut prev = 0;
        for sec in &s.sections {
            assert!(sec.start_bar >= prev, "sections must be in order");
            prev = sec.start_bar;
        }
        assert_eq!(s.section_start_secs(0), 0.0);
    }

    #[test]
    fn empty_tempo_is_byte_identical_to_constant() {
        // no `tempo` line ⇒ the funnels collapse to the constant-tempo formulas, exactly.
        let s = Score::builtin();
        assert!(s.tempo.is_empty());
        let sl = s.beat() / 4.0;
        for slot in [0, 1, 4, 16, 17, 64, 100] {
            assert!((s.slot_to_secs(slot as f32) - slot as f32 * sl).abs() < 1e-3);
            assert_eq!(s.secs_to_slot(s.slot_to_secs(slot as f32)), slot); // round-trips with the epsilon
        }
        // a constant-tempo score dumps NO tempo line (additivity invariant).
        assert!(!s.to_dsl().contains("tempo "));
    }

    #[test]
    fn tempo_map_warps_the_grid() {
        // bars 0,1 @120 (bar = 2.0s); bars 2,3 @60 (bar = 4.0s).
        let s = Score::from_str("bpm 120\ntempo @bar:2=60\nsection a 4 4\n").unwrap();
        assert_eq!(s.tempo, vec![TempoPoint { bar: 2, bpm: 60.0 }]);
        assert!((s.slot_to_secs(0.0) - 0.0).abs() < 1e-4);
        assert!((s.slot_to_secs(16.0) - 2.0).abs() < 1e-4); // end of bar 0
        assert!((s.slot_to_secs(32.0) - 4.0).abs() < 1e-4); // start of bar 2
        assert!((s.slot_to_secs(48.0) - 8.0).abs() < 1e-4); // bar 2 is 4 s long
        assert!((s.demo_len() - 12.0).abs() < 1e-4); // 2*2 + 2*4
        assert_eq!(s.secs_to_slot(4.0), 32); // exact downbeat floors forward, not one slot early
        // a beat in a slow bar is longer; an anchor lands at the warped wall-clock time.
        assert!((s.anchor_seconds("bar:2").unwrap() - 4.0).abs() < 1e-4);
        assert!((s.anchor_seconds("bar:3").unwrap() - 8.0).abs() < 1e-4);
        assert!((s.beat_at(5.0) - 1.0).abs() < 1e-4); // 5 s is in bar 2 (@60) → beat = 1.0 s
    }

    #[test]
    fn grid_sets_odd_meter_bar_length_and_round_trips() {
        // a 3/4 bar (grid:12) is 3 beats; a 4/4 bar (default 16) is 4. At 120 BPM a 16th = 0.125 s.
        let s = Score::from_str("bpm 120\nsection a 2 grid:12\nsection b 2\n").unwrap();
        assert_eq!((s.sections[0].grid, s.sections[1].grid), (12, 16));
        assert!(!s.uniform); // a non-16 grid takes the table path, not the fast path
        assert!((s.bar_start_secs(1) - 1.5).abs() < 1e-4); // bar 0 = 3/4 = 1.5 s
        assert!((s.bar_start_secs(2) - 3.0).abs() < 1e-4);
        assert!((s.bar_start_secs(3) - 5.0).abs() < 1e-4); // bar 2 = 4/4 = 2 s
        assert!((s.demo_len() - 7.0).abs() < 1e-4); // 1.5 + 1.5 + 2 + 2
        assert!((s.slot_to_secs(12.0) - 1.5).abs() < 1e-4); // 12 slots = end of the 3/4 bar
        assert!((s.slot_to_secs(24.0) - 3.0).abs() < 1e-4); // slot 24 = start of bar 2
        assert_eq!(s.secs_to_slot(3.0), 24); // exact boundary floors forward
        // round-trip preserves the grid; a default-16 section emits no `grid:` token.
        let b = Score::from_str(&s.to_dsl()).unwrap();
        assert_eq!(b.sections[0].grid, 12);
        assert!(s.to_dsl().contains("grid:12") && !s.to_dsl().contains("grid:16"));
        assert_eq!(s.to_dsl(), b.to_dsl());
    }

    #[test]
    fn tempo_map_round_trips_through_dsl() {
        let dsl = "bpm 120\ntempo @bar:2=60 @bar:4=90\nsection a 6 6\n";
        let a = Score::from_str(dsl).unwrap();
        let b = Score::from_str(&a.to_dsl()).unwrap();
        assert_eq!(a.tempo, b.tempo);
        assert_eq!(a.to_dsl(), b.to_dsl()); // idempotent
        for slot in [0.0, 16.0, 32.0, 48.0, 64.0, 96.0] {
            assert!((a.slot_to_secs(slot) - b.slot_to_secs(slot)).abs() < 1e-4);
        }
    }

    #[test]
    fn tempo_lints_and_errors() {
        // a point past the score's length warns (never fires).
        let s = Score::from_str("bpm 120\ntempo @bar:99=80\nsection a 4 4\n").unwrap();
        assert!(
            s.tempo_warnings()
                .iter()
                .any(|w| w.contains("never takes effect"))
        );
        // a bad tempo bpm is a hard parse error (matches the `bpm` guard).
        assert!(Score::from_str("bpm 120\ntempo @bar:2=0\nsection a 4 4\n").is_err());
        assert!(Score::from_str("bpm 120\ntempo 2=80\nsection a 4 4\n").is_err()); // missing `bar` prefix
    }

    #[test]
    fn anchor_seconds_resolves_every_form() {
        let s = Score::builtin();
        assert_eq!(s.anchor_seconds("start"), Some(0.0));
        // a real section name (the first) resolves to its start.
        let first = s.sections[0].name.clone();
        assert_eq!(s.anchor_seconds(&first), Some(0.0));
        // bar / beat forms, with and without the colon.
        assert_eq!(s.anchor_seconds("bar1"), Some(s.bar()));
        assert_eq!(s.anchor_seconds("bar:2"), Some(2.0 * s.bar()));
        assert_eq!(s.anchor_seconds("beat4"), Some(4.0 * s.beat()));
        // a plain number is seconds; whitespace + case are tolerated.
        assert_eq!(s.anchor_seconds("  2.5 "), Some(2.5));
        assert_eq!(s.anchor_seconds("nope"), None);
    }

    #[test]
    fn anchor_seconds_resolves_expression_offsets() {
        let s = Score::builtin();
        let bar = s.bar();
        let beat = s.beat();
        // section + bar/beat/second offsets (both signs), relative to a known cue.
        let drop = s
            .anchor_seconds("drop")
            .expect("builtin has a drop section");
        assert_eq!(s.anchor_seconds("drop+2bar"), Some(drop + 2.0 * bar));
        assert_eq!(s.anchor_seconds("drop-1beat"), Some(drop - beat));
        assert_eq!(s.anchor_seconds("drop+3s"), Some(drop + 3.0));
        assert_eq!(s.anchor_seconds("drop-1b"), Some(drop - bar)); // `b` = bar alias
        // offsets on bar/beat/second bases.
        assert_eq!(s.anchor_seconds("bar:4+2bar"), Some(6.0 * bar));
        assert_eq!(s.anchor_seconds("beat8-2"), Some(6.0 * beat)); // bare number = beats
        assert_eq!(s.anchor_seconds("start+1bar"), Some(bar));
        assert_eq!(s.anchor_seconds("10+5s"), Some(15.0));
        // a malformed suffix is NOT a valid offset → the whole token falls back to a section name.
        assert_eq!(s.anchor_seconds("drop+wat"), None);
    }

    #[test]
    fn anchor_kind_parses_the_spelling() {
        use AnchorKind::*;
        assert_eq!(AnchorKind::parse("start"), Some(Start));
        assert_eq!(AnchorKind::parse("bar:2"), Some(Bar(2.0)));
        assert_eq!(AnchorKind::parse("beat4"), Some(Beat(4.0)));
        assert_eq!(AnchorKind::parse(" 2.5 "), Some(Seconds(2.5)));
        assert_eq!(AnchorKind::parse("DROP"), Some(Section("drop".into()))); // case-folded name
        assert_eq!(AnchorKind::parse(""), None);
        // an unknown section is still a well-formed AnchorKind — it just resolves to None.
        let s = Score::builtin();
        assert_eq!(s.cue(&Section("nope".into())), None);
    }

    #[test]
    fn sharp_notes_parse_and_real_comments_still_strip() {
        // `#` is the comment char, but a mid-token `#` (the note F#5 / chord F#) must NOT be eaten —
        // and a genuine trailing `# comment` must still be stripped (else the lead line mis-counts).
        let dsl = "bpm 120\n\
                   chords F#\n\
                   section a 4 4\n\
                   a.lead p0: F#5 . . .  . . . .  . . . .  . . . .   # trailing comment with # inside\n";
        assert!(
            Score::from_str(dsl).is_ok(),
            "F#5 + a trailing comment should parse"
        );
        // a leading-`#` line is a comment (ignored); a bad note still errors.
        assert!(
            Score::from_str(
                "bpm 120\nchords G\nsection a 4 4\na.lead p0: Z9 . . . . . . . . . . . . . . .\n"
            )
            .is_err()
        );
    }

    #[test]
    fn malformed_magnitudes_error_instead_of_hanging_or_oom() {
        // A huge phase index would resize/index a lane Vec by ~1e9 (OOM) or overflow usize (panic);
        // a huge bar count would make the whole-track walks iterate bars·16 into an unbounded Vec;
        // bpm 0 makes beat()/demo_len() infinite. All must be rejected (→ fall back to the built-in).
        assert!(
            Score::from_str("bpm 120\nsection a 4 4\na.kick p999999999: x... .... .... ....")
                .is_err()
        );
        assert!(
            Score::from_str(
                "bpm 120\nsection a 4 4\na.kick p18446744073709551615: x... .... .... ...."
            )
            .is_err()
        );
        assert!(Score::from_str("bpm 120\nsection a 4000000000 4").is_err());
        assert!(Score::from_str("bpm 0\nsection a 4 4").is_err());
        // a sane phase + bar count still parses
        assert!(Score::from_str("bpm 120\nsection a 4 4\na.kick p1: x... .... .... ....").is_ok());
    }

    #[test]
    fn per_section_chords_override_the_global_progression() {
        // global = G major everywhere; the `verse` section flips to a G-minor `chords:` line. The
        // global chord at the verse's time must be the section's (minor), not the global (major).
        let dsl = "bpm 120\nchords G\n\
                   section intro 2 2\nsection verse 2 2\n\
                   verse.chords: Am\n";
        let s = Score::from_str(dsl).unwrap();
        let intro = s.chord_at(0.1); // intro → global G major
        let verse = s.chord_at(s.section_start_secs(1) + 0.1); // verse → section A minor
        assert!(!intro.minor, "intro uses the global G major");
        assert!(verse.minor, "verse uses its own A-minor override");
        assert!(
            (verse.root - note_freq("A3").unwrap()).abs() < 1.0,
            "verse root is A, not the global G"
        );
    }

    #[test]
    fn multi_bar_lead_phrase_advances_and_loops() {
        // a 2-bar phrase (32 tokens): bar0 has C5 at slot 0, bar1 has E5 at slot 0. Over a 4-bar
        // section the lead should play C5, E5, C5, E5 at each bar's downbeat (the phrase loops).
        let dsl = "bpm 120\nchords C\nsection a 4 4\n\
                   a.lead p0: C5 . . . . . . . . . . . . . . .  E5 . . . . . . . . . . . . . . .\n";
        let s = Score::from_str(dsl).unwrap();
        let notes = s.lead_notes();
        assert_eq!(notes.len(), 4, "4 downbeat notes over 4 bars");
        let c5 = note_freq("C5").unwrap();
        let e5 = note_freq("E5").unwrap();
        let pitch = |f: f32| {
            if (f - c5).abs() < 1.0 {
                'C'
            } else if (f - e5).abs() < 1.0 {
                'E'
            } else {
                '?'
            }
        };
        let seq: String = notes.iter().map(|&(_, f, _)| pitch(f)).collect();
        assert_eq!(seq, "CECE", "the phrase advances per bar and loops");
        // and the bar times line up with the grid.
        assert!((notes[1].0 - s.bar()).abs() < 1e-3);
    }

    #[test]
    fn tie_token_extends_note_hold() {
        // C4 held over 2 tie slots then a rest → ONE onset whose hold spans the two ties.
        let s =
            Score::from_str("bpm 120\nsection a 1\na.lead p0: C4 - - .  . . . .  . . . .  . . . .")
                .unwrap();
        let notes = s.lead_notes();
        assert_eq!(
            notes.len(),
            1,
            "the two ties extend the C4, they don't add onsets"
        );
        let (t, f, hold) = notes[0];
        assert_eq!(t, 0.0);
        assert!((f - note_freq("C4").unwrap()).abs() < 1.0, "C4, got {f}");
        assert!(
            (hold - 2.0 * (s.beat() / 4.0)).abs() < 1e-6,
            "2 tie slots held: {hold}"
        );
        // no ties → hold 0 (the render duration is then unchanged → byte-identical audio).
        let s0 =
            Score::from_str("bpm 120\nsection a 1\na.lead p0: C4 . . .  . . . .  . . . .  . . . .")
                .unwrap();
        assert_eq!(s0.lead_notes()[0].2, 0.0);
        // the shipped builtin score uses no ties → every note hold==0 (existing audio unchanged).
        assert!(
            Score::builtin()
                .lead_notes()
                .iter()
                .all(|&(_, _, h)| h == 0.0)
        );
    }

    #[test]
    fn single_bar_lead_still_repeats_every_bar() {
        // backward-compat: a 1-bar phrase plays the same bar every bar (the old behaviour).
        let dsl = "bpm 120\nchords C\nsection a 3 3\na.lead p0: G5 . . . . . . . . . . . . . . .\n";
        let s = Score::from_str(dsl).unwrap();
        assert_eq!(s.lead_notes().len(), 3); // G5 on every one of the 3 bars
    }

    #[test]
    fn validate_flags_phase_mismatch_and_ignored_melodic_phases() {
        // 8-bar section but phases 4,4 + fill = 9 (mismatch); a lead phrase written as p1 (no p0) is
        // both "p1+ ignored" AND "silent". All are WARNINGS — the score still parses.
        let dsl = "bpm 120\nchords C\nsection a 8 4,4 fill\n\
                   a.kick p5: x... .... .... ....\n\
                   a.lead p1: C5 . . . . . . . . . . . . . . .\n";
        let s = Score::from_str(dsl).expect("warnings, not errors — still parses");
        let w = validate(&s.sections);
        assert!(
            w.iter().any(|m| m.contains("bars but phases")),
            "phase/bar mismatch flagged: {w:?}"
        );
        assert!(
            w.iter().any(|m| m.contains(".kick`: defines p5")),
            "out-of-range drum phase flagged: {w:?}"
        );
        assert!(
            w.iter().any(|m| m.contains("p1+ phrases are ignored")),
            "ignored melodic phase flagged: {w:?}"
        );
        assert!(
            w.iter().any(|m| m.contains("SILENT")),
            "silent (p1-only) melodic lane flagged: {w:?}"
        );
        // and a clean score yields NO warnings.
        assert!(
            validate(&Score::builtin().sections).is_empty(),
            "the built-in score must be warning-clean"
        );
    }

    #[test]
    fn absurdly_long_score_warns_and_demo_len_is_capped() {
        // two 9000-bar sections = 18000 bars (each under the per-section MAX_BARS, but the SUM is
        // pathological) → L3: validate warns + demo_len() clamps so the synth can't try to allocate
        // tens of GB. Real tracks are tens of bars / ~90-170 s.
        let dsl = "bpm 120\nchords C\nsection a 9000 9000\nsection b 9000 9000\n";
        let s = Score::from_str(dsl).expect("parses (warnings, not errors)");
        let w = validate(&s.sections);
        assert!(
            w.iter()
                .any(|m| m.contains("far longer than any real track")),
            "aggregate length flagged: {w:?}"
        );
        assert_eq!(
            s.demo_len(),
            MAX_DEMO_SECS,
            "demo_len clamps to the ceiling (bounds the synth buffer allocation)"
        );
    }

    #[test]
    fn per_section_set_overrides_the_global_knob_and_round_trips() {
        let dsl = "bpm 120\nchords C\nsection intro 2 2\nsection drop 2 2\n\
                   set house=0.1\ndrop.set house=0.3 lead=0.9\n";
        let s = Score::from_str(dsl).unwrap();
        let drop_t = s.section_start_secs(1) + 0.1;
        assert!(
            (s.param("house", 0.0) - 0.1).abs() < 1e-6,
            "global house = 0.1"
        );
        assert!(
            (s.param_at(0.1, "house", 0.0) - 0.1).abs() < 1e-6,
            "intro falls back to the global 0.1"
        );
        assert!(
            (s.param_at(drop_t, "house", 0.0) - 0.3).abs() < 1e-6,
            "drop overrides house to 0.3"
        );
        assert!(
            (s.param_at(drop_t, "lead", 0.0) - 0.9).abs() < 1e-6,
            "drop also overrides lead"
        );
        // the override survives a to_dsl → from_str round-trip.
        let s2 = Score::from_str(&s.to_dsl()).unwrap();
        assert!(
            (s2.param_at(drop_t, "house", 0.0) - 0.3).abs() < 1e-6,
            "the per-section override round-trips through to_dsl"
        );
    }

    #[test]
    fn per_section_fx_list_overrides_the_name_defaults_and_round_trips() {
        // no `fx:` line → the built-in name-based defaults (a drop gets the wall + jet, not casio).
        let plain = Score::from_str("bpm 120\nchords C\nsection drop 4 4\n").unwrap();
        assert!(plain.fx_on("drop", "wall"));
        assert!(plain.fx_on("drop", "jet"));
        assert!(!plain.fx_on("drop", "casio"));
        // an explicit `<section>.fx:` line is authoritative: keep the wall, drop the jet.
        let custom =
            Score::from_str("bpm 120\nchords C\nsection drop 4 4\ndrop.fx: wall house\n").unwrap();
        assert!(custom.fx_on("drop", "wall"));
        assert!(custom.fx_on("drop", "house"));
        assert!(
            !custom.fx_on("drop", "jet"),
            "explicit fx list omits the jet"
        );
        // round-trips through to_dsl.
        let s2 = Score::from_str(&custom.to_dsl()).unwrap();
        assert!(s2.fx_on("drop", "wall") && !s2.fx_on("drop", "jet"));
    }

    #[test]
    fn malformed_pattern_line_errors_instead_of_panicking() {
        // a line starting with ':' that still looks like a pattern (contains '.') leaves an empty
        // target before the ':' — that used to `.unwrap()`-panic; now it's a clean parse error.
        assert!(
            Score::from_str("bpm 120\nchords C\nsection a 4 4\n:a.b x...\n").is_err(),
            "malformed pattern line should Err, not panic"
        );
        // a bare keyword-less colon line is just an unknown keyword (also a clean Err).
        assert!(Score::from_str("bpm 120\nchords C\nsection a 4 4\n: nope\n").is_err());
    }

    #[test]
    fn drum_hits_are_ordered_and_in_range() {
        let s = Score::builtin();
        let hits = s.hits(Inst::Kick);
        assert!(!hits.is_empty(), "the built-in track should kick");
        assert!(hits.windows(2).all(|w| w[0] <= w[1]), "hits in time order");
        assert!(hits.iter().all(|&t| t >= 0.0 && t <= s.demo_len()));
    }
}
