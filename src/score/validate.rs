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
