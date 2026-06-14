//! Music score — the *composition*, data-driven. Ported from Cinder's (Kristian Vlaardingerbroek,
//! deFEEST) `term-demo` (MIT, Outline 2026): the BPM→beat→bar grid, the section timeline
//! (intro→build→drop→breakdown→climax→outro), the drum patterns and the per-section dynamics that
//! the synth (`audio.rs`) and the visual `@@anchor`s both read.
//!
//! The music lives in a **text file**, not in code: `assets/score.txt` (a tracker-DSL score) is
//! loaded by default — edit it, no recompile — and `include_str!`'d as the embedded fallback for a
//! bundled binary (so the notes/patterns/chords are not duplicated in Rust). `MARTIN_SCORE=<file>`
//! overrides it; `MARTIN_SCORE_DUMP=<file>` writes a copy. The *instrument* (how a kick/stab
//! sounds) stays in `audio.rs`. 16 steps per bar (16th notes).
//!
//! This module is split by concern: `types` (the data), `parse` (text → `Score`), `dump`
//! (`Score` → text), `validate` (the structural lint), and the timeline maths on `Score` below.

mod dump;
mod parse;
mod types;
mod validate;

pub use types::{Chord, Inst, Levels, NoteLane, Ramp, Section};

const SLOTS_PER_BAR: i64 = 16;
const BEATS_PER_BAR: f32 = 4.0;

/// The editable default score, loaded from disk when present (so editing it needs no recompile).
/// The same file is `include_str!`'d as the embedded fallback — the music lives here, not in code.
const DEFAULT_SCORE: &str = "assets/score.txt";

/// Crossfade window (seconds) smoothing per-section dynamics steps at boundaries — long enough to
/// kill the click, short enough not to smear the musical transition.
pub const SECTION_FADE: f32 = 0.12;

/// A whole score: tempo + an ordered list of sections (which carry their own patterns + dynamics).
#[derive(Clone)]
pub struct Score {
    pub bpm: f32,
    pub chords: Vec<Chord>, // per-bar chord progression (cycles); drives bass + stab
    pub sections: Vec<Section>,
    total_bars: u32,
    /// Free-form mix/fx knobs from `set <key>=<value>` lines — the synth reads these (with built-in
    /// defaults) so the SOUND can be tuned by editing the score file (no recompile), not the engine.
    params: std::collections::HashMap<String, f32>,
}

impl Score {
    /// Lay out the sections (cumulative `start_bar`, total length) — the single place section
    /// timing is derived, so the file and the built-in agree.
    fn new(bpm: f32, chords: Vec<Chord>, mut sections: Vec<Section>) -> Self {
        let mut bar = 0;
        for s in &mut sections {
            s.start_bar = bar;
            bar += s.bars;
        }
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
            chords,
            sections,
            total_bars: bar,
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
    pub fn beat(&self) -> f32 {
        60.0 / self.bpm
    }
    pub fn bar(&self) -> f32 {
        BEATS_PER_BAR * self.beat()
    }
    fn slot_len(&self) -> f32 {
        self.beat() / 4.0
    }
    pub fn demo_len(&self) -> f32 {
        self.total_bars as f32 * self.bar()
    }

    fn abs_slot(&self, t: f32) -> i64 {
        let sl = self.slot_len();
        ((t + sl * 1e-3) / sl).floor() as i64
    }
    fn bar_idx_at(&self, t: f32) -> u32 {
        (self.abs_slot(t).max(0) / SLOTS_PER_BAR) as u32
    }

    // --- sections -----------------------------------------------------------------------------
    fn section_index_at(&self, t: f32) -> usize {
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
        self.sections[idx].start_bar as f32 * self.bar()
    }

    // --- patterns -----------------------------------------------------------------------------
    fn lane_hits(&self, inst: Inst, t: f32) -> [bool; 16] {
        let i = self.section_index_at(t);
        let s = &self.sections[i];
        let into = (self.bar_idx_at(t) as i64 - s.start_bar as i64).max(0) as u32;
        s.lane(inst).at(s.phase_at(into))
    }

    /// Every hit time (s) for `inst` across the whole track, in order — the synth builds a voice at
    /// each. Forward enumeration: walk every 16th-note slot and keep the ones that fire.
    pub fn hits(&self, inst: Inst) -> Vec<f32> {
        let sl = self.slot_len();
        let slots = self.total_bars as i64 * SLOTS_PER_BAR;
        (0..slots)
            .filter_map(|s| {
                let t = s as f32 * sl;
                self.lane_hits(inst, t)[(s % SLOTS_PER_BAR) as usize].then_some(t)
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

    fn note_grid(&self, t: f32, pick: fn(&Section) -> &NoteLane) -> [Option<f32>; 16] {
        let i = self.section_index_at(t);
        let s = &self.sections[i];
        let into = (self.bar_idx_at(t) as i64 - s.start_bar as i64).max(0) as u32;
        let is_fill = s.fill && into == s.bars - 1;
        pick(s).bar(into, is_fill)
    }

    /// Every note of a note-lane as (time, freq) across the whole track — the synth builds a voice
    /// at each onset.
    fn note_line(&self, pick: fn(&Section) -> &NoteLane) -> Vec<(f32, f32)> {
        let sl = self.slot_len();
        let slots = self.total_bars as i64 * SLOTS_PER_BAR;
        (0..slots)
            .filter_map(|s| {
                let t = s as f32 * sl;
                self.note_grid(t, pick)[(s % SLOTS_PER_BAR) as usize].map(|f| (t, f))
            })
            .collect()
    }

    /// The `lead` (foreground melody) onsets.
    pub fn lead_notes(&self) -> Vec<(f32, f32)> {
        self.note_line(|s| &s.lead)
    }

    /// The `arp` (second melodic line) onsets.
    pub fn arp_notes(&self) -> Vec<(f32, f32)> {
        self.note_line(|s| &s.arp)
    }

    /// The `bass` (articulated bassline) onsets — empty unless the score writes a `bass` lane.
    pub fn bass_notes(&self) -> Vec<(f32, f32)> {
        self.note_line(|s| &s.bass)
    }

    // --- dynamics -----------------------------------------------------------------------------
    fn section_value<F: Fn(&Section) -> Ramp>(&self, t: f32, pick: &F) -> f32 {
        let i = self.section_index_at(t);
        let s = &self.sections[i];
        let dur = (s.bars as f32 * self.bar()).max(1e-3);
        let p = ((t - self.section_start_secs(i)) / dur).clamp(0.0, 1.0);
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

    /// Master gain (per-section, crossfaded) the synth multiplies the mix by.
    pub fn gain_at(&self, t: f32) -> f32 {
        self.smooth(t, |s| s.gain)
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
            AnchorKind::Bar(b) => Some(b * self.bar()),
            AnchorKind::Beat(b) => Some(b * self.beat()),
            AnchorKind::Seconds(s) => Some(*s),
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
}

impl AnchorKind {
    /// Classify an anchor token. Order: `start` → `bar…`/`beat…` → a bare number → otherwise a
    /// section name. (Section names like `intro`/`drop` never collide with the numeric/prefix forms.)
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
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
        Some(Self::Section(s))
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
        let seq: String = notes.iter().map(|&(_, f)| pitch(f)).collect();
        assert_eq!(seq, "CECE", "the phrase advances per bar and loops");
        // and the bar times line up with the grid.
        assert!((notes[1].0 - s.bar()).abs() < 1e-3);
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
