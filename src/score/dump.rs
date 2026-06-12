//! Serialise a `Score` back to the tracker DSL (`MARTIN_SCORE_DUMP`) — the inverse of `parse`. It
//! round-trips: `from_str(score.to_dsl())` reproduces the score. The note/chord/pattern formatters
//! live here too (the parsers are in `parse`).

use super::Score;
use super::types::{Chord, Inst, NoteLane, Ramp, Section};

impl Score {
    /// Serialize back to the tracker DSL — `MARTIN_SCORE_DUMP` writes the built-in this way for a
    /// ready-to-edit starting file (and it round-trips through `from_str`).
    pub fn to_dsl(&self) -> String {
        let mut o = String::new();
        o.push_str("# martin score — tracker DSL. Edit + load with MARTIN_SCORE=<this file>.\n");
        o.push_str(&format!("bpm {}\n", fnum(self.bpm)));
        o.push_str(&format!(
            "chords {}\n\n",
            self.chords
                .iter()
                .map(chord_str)
                .collect::<Vec<_>>()
                .join(" ")
        ));
        if !self.params.is_empty() {
            let mut kv: Vec<_> = self.params.iter().collect();
            kv.sort_by(|a, b| a.0.cmp(b.0));
            o.push_str("# mix/fx knobs (tune the SOUND here, no recompile — synth reads these).\n");
            o.push_str("set ");
            o.push_str(
                &kv.iter()
                    .map(|(k, v)| format!("{k}={}", fnum(**v)))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            o.push_str("\n\n");
        }
        o.push_str("# section <name> <bars> <phase-bars,csv> [fill]\n");
        for s in &self.sections {
            let ph = s
                .phases
                .iter()
                .map(|p| p.to_string())
                .collect::<Vec<_>>()
                .join(",");
            o.push_str(&format!(
                "section {} {} {}{}\n",
                s.name,
                s.bars,
                ph,
                if s.fill { " fill" } else { "" }
            ));
        }
        for s in &self.sections {
            if !s.chords.is_empty() {
                o.push_str(&format!(
                    "{}.chords: {}\n",
                    s.name,
                    s.chords.iter().map(chord_str).collect::<Vec<_>>().join(" ")
                ));
            }
        }
        for s in &self.sections {
            if !s.params.is_empty() {
                let mut kv: Vec<_> = s.params.iter().collect();
                kv.sort_by(|a, b| a.0.cmp(b.0));
                o.push_str(&format!(
                    "{}.set {}\n",
                    s.name,
                    kv.iter()
                        .map(|(k, v)| format!("{k}={}", fnum(**v)))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            }
        }
        for s in &self.sections {
            if let Some(fx) = &s.fx {
                o.push_str(&format!("{}.fx: {}\n", s.name, fx.join(" ")));
            }
        }
        o.push_str(
            "\n# patterns: <section>.<kick|snare|hat|stab> p<N>|fill: 16 steps (x=hit .=rest)\n",
        );
        for s in &self.sections {
            for (inst, name) in [
                (Inst::Kick, "kick"),
                (Inst::Snare, "snare"),
                (Inst::Hat, "hat"),
                (Inst::Stab, "stab"),
            ] {
                let lane = s.lane(inst);
                for (p, grid) in lane.phases.iter().enumerate() {
                    if lane.any(grid) {
                        o.push_str(&format!("{}.{name} p{p}: {}\n", s.name, pat_str(grid)));
                    }
                }
                if s.fill && lane.any(&lane.fill) {
                    o.push_str(&format!(
                        "{}.{name} fill: {}\n",
                        s.name,
                        pat_str(&lane.fill)
                    ));
                }
            }
        }
        o.push_str(
            "\n# melody: <section>.lead p<N>|fill: 16 note slots (A4 C#5 . E5 …; . = rest)\n",
        );
        for s in &self.sections {
            for (p, grid) in s.lead.phases.iter().enumerate() {
                if NoteLane::any(grid) {
                    o.push_str(&format!("{}.lead p{p}: {}\n", s.name, notes_phrase(grid)));
                }
            }
            if NoteLane::any(&s.lead.fill) {
                o.push_str(&format!(
                    "{}.lead fill: {}\n",
                    s.name,
                    notes_phrase(&s.lead.fill)
                ));
            }
        }
        o.push_str("\n# arp: <section>.arp p<N>|fill — a second melodic line, same note grammar\n");
        for s in &self.sections {
            for (p, grid) in s.arp.phases.iter().enumerate() {
                if NoteLane::any(grid) {
                    o.push_str(&format!("{}.arp p{p}: {}\n", s.name, notes_phrase(grid)));
                }
            }
            if NoteLane::any(&s.arp.fill) {
                o.push_str(&format!(
                    "{}.arp fill: {}\n",
                    s.name,
                    notes_phrase(&s.arp.fill)
                ));
            }
        }
        o.push_str(
            "\n# bass: <section>.bass p<N>|fill — an articulated bassline, same note grammar\n",
        );
        for s in &self.sections {
            for (p, grid) in s.bass.phases.iter().enumerate() {
                if NoteLane::any(grid) {
                    o.push_str(&format!("{}.bass p{p}: {}\n", s.name, notes_phrase(grid)));
                }
            }
            if NoteLane::any(&s.bass.fill) {
                o.push_str(&format!(
                    "{}.bass fill: {}\n",
                    s.name,
                    notes_phrase(&s.bass.fill)
                ));
            }
        }
        o.push_str(
            "\n# dynamics 0..1 per section (`v` constant or `a>b` ramp across the section)\n",
        );
        for (kw, pick) in [
            ("gain", &(|s: &Section| s.gain) as &dyn Fn(&Section) -> Ramp),
            ("sub", &(|s: &Section| s.sub)),
            ("mids", &(|s: &Section| s.mids)),
        ] {
            o.push_str(kw);
            for s in &self.sections {
                o.push_str(&format!(" {} {}", s.name, ramp_str(&pick(s))));
            }
            o.push('\n');
        }
        o
    }
}

// ---- formatters ----------------------------------------------------------------------------

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

fn freq_to_midi(freq: f32) -> i32 {
    (69.0 + 12.0 * (freq / 440.0).log2()).round() as i32
}

/// Nearest note name (with octave) for a frequency — for `to_dsl`.
fn note_name(freq: f32) -> String {
    let midi = freq_to_midi(freq);
    format!(
        "{}{}",
        NOTE_NAMES[midi.rem_euclid(12) as usize],
        midi.div_euclid(12) - 1
    )
}

fn notes_str(g: &[Option<f32>; 16]) -> String {
    let toks: Vec<String> = g
        .iter()
        .map(|n| n.map(note_name).unwrap_or_else(|| ".".into()))
        .collect();
    toks.chunks(4)
        .map(|c| c.join(" "))
        .collect::<Vec<_>>()
        .join("  ")
}

/// A whole melodic phrase (1+ bars) on one line — each bar's `notes_str`, joined by 3 spaces.
fn notes_phrase(phrase: &[[Option<f32>; 16]]) -> String {
    phrase.iter().map(notes_str).collect::<Vec<_>>().join("   ")
}

fn chord_str(c: &Chord) -> String {
    let name = note_name(c.root); // e.g. "A3"
    let letter: String = name.chars().take_while(|ch| !ch.is_ascii_digit()).collect();
    format!("{letter}{}", if c.minor { "m" } else { "" })
}

fn pat_str(p: &[bool; 16]) -> String {
    let mut s = String::with_capacity(19);
    for (i, &b) in p.iter().enumerate() {
        if i > 0 && i % 4 == 0 {
            s.push(' ');
        }
        s.push(if b { 'x' } else { '.' });
    }
    s
}

fn fnum(v: f32) -> String {
    format!("{v}")
}

fn ramp_str(r: &Ramp) -> String {
    if (r.a - r.b).abs() < 1e-6 {
        fnum(r.a)
    } else {
        format!("{}>{}", fnum(r.a), fnum(r.b))
    }
}
