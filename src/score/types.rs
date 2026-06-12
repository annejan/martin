//! The score's data types: the section timeline, the per-section drum/note lanes, chords, dynamics
//! ramps, and the enveloped levels the synth reads. Pure data — parsing is in `parse`, the text dump
//! in `dump`, the structural lint in `validate`, and the timeline maths on `Score` in `mod`.

/// The four sequenced drum/voice lanes (the *instrument* synthesis lives in `audio.rs`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Inst {
    Kick,
    Snare,
    Hat,
    Stab,
}

/// A per-section value that ramps linearly `a → b` across the section (`a == b` = constant). Used
/// for gain / sub / mids so a section can build (e.g. the riser into the drop) instead of stepping.
#[derive(Clone, Copy)]
pub struct Ramp {
    pub a: f32,
    pub b: f32,
}

impl Ramp {
    pub(super) fn new(a: f32, b: f32) -> Self {
        Self { a, b }
    }
    pub(super) fn c(v: f32) -> Self {
        Self { a: v, b: v }
    }
    /// value at progress `p` (0..1) through the section.
    pub(super) fn at(&self, p: f32) -> f32 {
        self.a + (self.b - self.a) * p
    }
}

/// One instrument's pattern within a section: a 16-step grid per phase, plus the fill-bar grid.
#[derive(Clone, Default)]
pub struct Lane {
    pub phases: Vec<[bool; 16]>,
    pub fill: [bool; 16],
}

impl Lane {
    /// the grid for `phase` (255 = fill). An undefined phase is **silent** — lanes only carry the
    /// phases that have hits, so this keeps `MARTIN_SCORE_DUMP` → reload faithful and makes
    /// "didn't write a pattern" mean "doesn't play" (not "repeat the previous one").
    pub(super) fn at(&self, phase: u8) -> [bool; 16] {
        if phase == 255 {
            self.fill
        } else {
            self.phases
                .get(phase as usize)
                .copied()
                .unwrap_or([false; 16])
        }
    }
    pub(super) fn any(&self, p: &[bool; 16]) -> bool {
        p.iter().any(|&b| b)
    }
}

/// A melodic note lane: a frequency (Hz) per 16-step slot (`None` = rest) — same phase/fill shape
/// as `Lane`, but pitched. This is the `lead` (melody) the synth plays.
#[derive(Clone, Default)]
pub struct NoteLane {
    /// Each phase is a melodic **phrase**: a sequence of 1+ bars that plays out and loops every N
    /// bars (a 1-bar phrase = the old per-bar repeat). So a section can carry a real multi-bar
    /// melody — a through-composed line — not just one looping bar.
    pub phases: Vec<Vec<[Option<f32>; 16]>>,
    pub fill: Vec<[Option<f32>; 16]>,
}

impl NoteLane {
    /// The 16-slot grid `into` bars into the section. The melodic phrase (the primary `p0` line)
    /// loops CONTINUOUSLY across the whole section — independent of the drum phases and the fill
    /// bar — so a through-composed line plays as one uninterrupted statement (and breathes over a
    /// drum fill) instead of being chopped/restarted at every phase boundary.
    ///
    /// When `fill_active` is true and the lane has a fill phrase, that fill is returned instead of
    /// the looped p0 phrase — so a per-section `<section>.lead fill: <16 rests>` silences the
    /// melody on the drum-fill bar, preventing the verse's pickup from bleeding into the chorus.
    pub(super) fn bar(&self, into: u32, fill_active: bool) -> [Option<f32>; 16] {
        if fill_active && !self.fill.is_empty() {
            return self.fill[(into as usize).min(self.fill.len() - 1)];
        }
        let phrase = self.phases.first().map(Vec::as_slice).unwrap_or(&[]);
        if phrase.is_empty() {
            [None; 16]
        } else {
            phrase[into as usize % phrase.len()]
        }
    }
    pub(super) fn any(phrase: &[[Option<f32>; 16]]) -> bool {
        phrase.iter().flatten().any(|n| n.is_some())
    }
}

/// A chord: a root frequency + major/minor quality. Cycles per bar (the `chords` line) and drives
/// the bass + stab, so the harmony moves under the melody.
#[derive(Clone, Copy)]
pub struct Chord {
    pub root: f32,
    pub minor: bool,
}

impl Chord {
    /// (root, third, fifth) triad frequencies.
    pub fn triad(&self) -> [f32; 3] {
        let third = if self.minor { 3.0 } else { 4.0 };
        [self.root, self.root * semis(third), self.root * semis(7.0)]
    }
}

/// Frequency ratio of `n` semitones.
fn semis(n: f32) -> f32 {
    2f32.powf(n / 12.0)
}

/// The built-in, name-based FX/layer gating — the behaviour a section gets when it has NO explicit
/// `<section>.fx:` line (so the shipped op-de-camping score, which has none, is unchanged). The layer
/// names (`wall`/`shimmer`/`donk`/`house`/`casio`) and transition accents (`riser`/`jet`/`impact`/
/// `bang`) each fire in the sections the synth used to hard-code.
fn default_fx(name: &str, token: &str) -> bool {
    let any = |names: &[&str]| names.contains(&name);
    match token {
        "wall" => any(&["drop", "climax", "outro"]),
        "shimmer" => any(&["climax", "outro"]),
        "donk" => any(&["drop", "climax"]),
        "house" => any(&["drop", "climax", "outro"]),
        "casio" => any(&["outro"]),
        "riser" => any(&["build", "drop", "climax", "outro"]),
        "jet" => any(&["drop", "climax"]),
        "impact" => any(&["drop", "breakdown", "climax"]),
        "bang" => any(&["outro"]),
        _ => false,
    }
}

/// One section of the arrangement: a span of `bars` divided into `phases` (bars per phase) with an
/// optional fill bar, its dynamics curves, and its four drum lanes.
#[derive(Clone)]
pub struct Section {
    pub name: String,
    pub bars: u32,
    pub phases: Vec<u32>, // bars per phase; if `fill`, the final bar of the section is the fill
    pub fill: bool,
    pub gain: Ramp,
    pub sub: Ramp,
    pub mids: Ramp,
    pub kick: Lane,
    pub snare: Lane,
    pub hat: Lane,
    pub stab: Lane,
    pub lead: NoteLane,     // melody (one note per slot); empty = no lead
    pub arp: NoteLane,      // a second melodic line (the plucky counter-melody); empty = no arp
    pub bass: NoteLane, // an articulated bassline (one note per slot); empty = chord-root sub only
    pub chords: Vec<Chord>, // per-section chord override (cycles within the section); empty = global
    /// Per-section mix/fx knob overrides (`<section>.set key=value`): when set, `param_at` returns
    /// these inside this section instead of the global `set` value — e.g. a louder house organ in the
    /// drop without touching the climax. Empty = use the global knob.
    pub params: std::collections::HashMap<String, f32>,
    /// Per-section FX/layer selection (`<section>.fx: wall jet …`). `None` = use the built-in
    /// name-based defaults (so the shipped demo is unchanged); `Some` = exactly these accents, letting
    /// a different genre opt out of e.g. the demoscene jets without renaming its sections. See `fx_on`.
    pub fx: Option<Vec<String>>,
    pub start_bar: u32, // computed by Score::new
}

impl Section {
    pub(super) fn empty(name: String, bars: u32, phases: Vec<u32>, fill: bool) -> Self {
        Self {
            name,
            bars,
            phases,
            fill,
            gain: Ramp::c(0.85),
            sub: Ramp::c(0.5),
            mids: Ramp::c(0.6),
            kick: Lane::default(),
            snare: Lane::default(),
            hat: Lane::default(),
            stab: Lane::default(),
            lead: NoteLane::default(),
            arp: NoteLane::default(),
            bass: NoteLane::default(),
            chords: Vec::new(),
            params: std::collections::HashMap::new(),
            fx: None,
            start_bar: 0,
        }
    }

    /// Whether this section gets the FX/layer `token` (`wall`/`shimmer`/`donk`/`house`/`casio` layers,
    /// `riser`/`jet`/`impact`/`bang` transitions). An explicit `<section>.fx:` list is authoritative;
    /// otherwise the built-in name-based default fires (so a score with no `fx:` lines is unchanged).
    pub fn fx_on(&self, token: &str) -> bool {
        match &self.fx {
            Some(list) => list.iter().any(|t| t == token),
            None => default_fx(&self.name, token),
        }
    }

    pub(super) fn lane(&self, inst: Inst) -> &Lane {
        match inst {
            Inst::Kick => &self.kick,
            Inst::Snare => &self.snare,
            Inst::Hat => &self.hat,
            Inst::Stab => &self.stab,
        }
    }

    pub(super) fn lane_mut(&mut self, inst: &str) -> Option<&mut Lane> {
        match inst {
            "kick" => Some(&mut self.kick),
            "snare" => Some(&mut self.snare),
            "hat" => Some(&mut self.hat),
            "stab" => Some(&mut self.stab),
            _ => None,
        }
    }

    /// Which phase a bar `into` this section is in: the trailing bar is the fill (255) when the
    /// section has one; otherwise the phase whose cumulative bar-span contains `into`.
    pub(super) fn phase_at(&self, into: u32) -> u8 {
        self.phase_and_offset(into).0
    }

    /// The phase index AND how many bars into that phase `into` is — so a multi-bar melodic phrase
    /// knows which of its bars to play. The trailing fill bar is `(255, 0)`.
    fn phase_and_offset(&self, into: u32) -> (u8, u32) {
        if self.fill {
            let total: u32 = self.phases.iter().sum::<u32>() + 1;
            if into >= total.saturating_sub(1) {
                return (255, 0);
            }
        }
        let mut acc = 0;
        for (i, &p) in self.phases.iter().enumerate() {
            if into < acc + p {
                return (i as u8, into - acc);
            }
            acc += p;
        }
        // past the defined phases → the last phase, offset from its start.
        let last = self.phases.len().saturating_sub(1);
        let before: u32 = self.phases.iter().take(last).sum();
        (last as u8, into.saturating_sub(before))
    }
}

/// The enveloped sub-bass / mids levels at a moment — the synth reads these for its osc + stab
/// amplitudes.
#[derive(Clone, Copy)]
pub struct Levels {
    pub sub_bass: f32,
    pub mids: f32,
}
