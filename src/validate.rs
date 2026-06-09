//! `MARTIN_VALIDATE=1`: a dry run. Parse the whole show and print the resolved timeline — every
//! part with its cue start time + effects, the compose stage, and the camera track — then exit
//! without building the renderer. A fast "is my show right, and what does it look like on paper?"
//! check; pairs with the parse diagnostics (unknown transitions/heads warn on stderr).

use crate::scene::compose::Composed;
use crate::scene::sequence::{part_starts, show_end, Sequence};
use crate::score::Score;
use crate::waypoints::{is_track, Waypoints};

/// Print the parsed show to stdout. Called from `main` before the app is built when MARTIN_VALIDATE
/// is set; `main` then returns (no window, no render).
pub fn report(seq: &Sequence, compose: &[Composed], cam: &Waypoints, score: &Score) {
    println!("\nmartin — show validation (dry run, no render)\n");
    println!(
        "score:    {} sections, ~{:.0}s",
        score.sections.len(),
        score.demo_len()
    );

    if !seq.parts.is_empty() {
        let starts = part_starts(&seq.parts);
        let end = show_end(&seq.parts, &starts);
        println!(
            "\nsequence: {} parts, ~{:.0}s total, {} gaussians/part",
            seq.parts.len(),
            end,
            seq.count
        );
        for (i, (part, &t)) in seq.parts.iter().zip(&starts).enumerate() {
            let tr = part
                .transition
                .map(|x| format!(" ~{x:?}"))
                .unwrap_or_default();
            let de = part.deform.map(|x| format!(" ^{x:?}")).unwrap_or_default();
            let out = part.out.map(|x| format!(" out:{x:?}")).unwrap_or_default();
            let cl = part
                .cluster
                .map(|n| format!(" cluster:{n}"))
                .unwrap_or_default();
            let anchored = if part.anchor.is_some() { " @@" } else { "" };
            println!(
                "  [{i:02}] t={t:6.1}s{anchored}  {}{tr}{de}{out}{cl}  (hold {:.1}, morph {:.1})",
                part.content.label(),
                part.hold,
                part.morph,
            );
        }
    }

    if !compose.is_empty() {
        println!("\ncompose:  {} objects", compose.len());
        for o in compose {
            println!("  {}", o.summary());
        }
    }

    if !cam.list.is_empty() {
        let kind = if is_track(&cam.list) {
            "track (music-timed)"
        } else {
            "path"
        };
        println!("\ncamera:   {} waypoints ({kind})", cam.list.len());
        for w in &cam.list {
            let t = w.t.map(|t| format!("t={t:6.1}s  ")).unwrap_or_default();
            println!(
                "  {t}dist {:.2}  yaw {:.2}  pitch {:.2}  target ({:.1},{:.1},{:.1})",
                w.dist, w.yaw, w.pitch, w.target.x, w.target.y, w.target.z,
            );
        }
    }
    println!();
}
