//! Structural lint of a parsed score + the `MARTIN_SCORE_STRICT` gate. Warnings, not errors: a bad
//! score should never stop the show (the loader in `parse` logs them, or aborts under strict mode).

use super::types::{NoteLane, Section};

/// `MARTIN_SCORE_STRICT` set (and not `0`/empty) → treat score warnings as fatal (for authoring + CI,
/// so a phase/bar typo can't silently ship). Unset → warnings are logged but the show still plays.
pub(super) fn strict_scores() -> bool {
    std::env::var("MARTIN_SCORE_STRICT")
        .map(|v| !v.is_empty() && v != "0")
        .unwrap_or(false)
}

/// Structural lint of the parsed sections — returns human-readable WARNINGS (not errors: a bad score
/// should never stop the show). It catches the silent traps the DSL otherwise hides:
///   • a phase/bar-count mismatch — the classic one (`section x 8 4,4 fill` → 4+4+1≠8): the extra or
///     missing bars just repeat the last phase, producing dead/duplicated bars with no error.
///   • a drum pattern on a phase the section doesn't have (`x.kick p5` when x has 3 phases) — it
///     never plays.
///   • a melodic `p1`+ phrase — note lanes loop `p0` CONTINUOUSLY across the section (see
///     `NoteLane::bar`), so any `p1`+ is silently ignored, and a lane with ONLY a `p1` is dead silent.
pub(super) fn validate(sections: &[Section]) -> Vec<String> {
    let mut w = Vec::new();
    for s in sections {
        let declared = s.phases.iter().sum::<u32>() + u32::from(s.fill);
        if declared != s.bars {
            w.push(format!(
                "section `{}`: {} bars but phases{} sum to {} — the extra/missing bars repeat the \
                 last phase (likely a typo)",
                s.name,
                s.bars,
                if s.fill { " + fill" } else { "" },
                declared
            ));
        }
        for (inst, lane) in [
            ("kick", &s.kick),
            ("snare", &s.snare),
            ("hat", &s.hat),
            ("stab", &s.stab),
        ] {
            if lane.phases.len() > s.phases.len() {
                w.push(format!(
                    "`{}.{inst}`: defines p{} but section `{}` has only {} phase(s) — that pattern \
                     never plays",
                    s.name,
                    lane.phases.len() - 1,
                    s.name,
                    s.phases.len()
                ));
            }
        }
        for (lname, lane) in [("lead", &s.lead), ("arp", &s.arp), ("bass", &s.bass)] {
            if lane.phases.iter().skip(1).any(|p| NoteLane::any(p)) {
                w.push(format!(
                    "`{}.{lname}`: p1+ phrases are ignored — melodic lanes loop p0 continuously \
                     across the section",
                    s.name
                ));
            }
            if lane.phases.len() > 1 && lane.phases.first().is_some_and(|p| !NoteLane::any(p)) {
                w.push(format!(
                    "`{}.{lname}`: no p0 phrase (only p1+), so the lane is SILENT — melodic lanes \
                     play p0; rename your phrase to p0",
                    s.name
                ));
            }
        }
    }
    w
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sec(name: &str, bars: u32, phases: Vec<u32>, fill: bool) -> Section {
        Section::empty(name.to_string(), bars, phases, fill)
    }

    /// A melodic phrase (one bar) that actually plays a note (vs. all-rests).
    fn note_phrase() -> Vec<[Option<f32>; 16]> {
        let mut g = [None; 16];
        g[0] = Some(440.0);
        vec![g]
    }

    #[test]
    fn clean_section_has_no_warnings() {
        assert!(validate(&[sec("intro", 4, vec![4], false)]).is_empty());
        // phases + fill summing to bars is also clean
        assert!(validate(&[sec("verse", 8, vec![4, 3], true)]).is_empty());
    }

    #[test]
    fn bar_count_mismatch_warns() {
        // the classic trap: `section x 8 4,4 fill` → 4+4+1 = 9 ≠ 8
        let w = validate(&[sec("x", 8, vec![4, 4], true)]);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("`x`") && w[0].contains("8 bars") && w[0].contains("sum to 9"));
    }

    #[test]
    fn drum_phase_beyond_section_warns() {
        let mut s = sec("x", 4, vec![4], false); // section has ONE phase
        s.kick.phases = vec![[false; 16], [true; 16]]; // …but kick defines a p1 → never plays
        let w = validate(&[s]);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("`x.kick`") && w[0].contains("never plays"));
    }

    #[test]
    fn melodic_p1_phrase_is_flagged_ignored() {
        let mut s = sec("x", 4, vec![4], false);
        s.lead.phases = vec![note_phrase(), note_phrase()]; // p0 + a p1 that's silently dropped
        let w = validate(&[s]);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("`x.lead`") && w[0].contains("p1+ phrases are ignored"));
    }

    #[test]
    fn melodic_only_p1_is_flagged_silent() {
        let mut s = sec("x", 4, vec![4], false);
        // p0 is all-rests, the real line sits in p1 → the whole lane plays SILENT
        s.bass.phases = vec![vec![[None; 16]], note_phrase()];
        let w = validate(&[s]);
        assert!(w.iter().any(|m| m.contains("SILENT")));
        assert!(w.iter().any(|m| m.contains("p1+ phrases are ignored")));
    }
}
