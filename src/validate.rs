//! `MARTIN_VALIDATE=1`: a dry run. Parse the whole show and print the resolved timeline — every
//! part with its cue start time + effects, the compose stage, and the camera track — then exit
//! without building the renderer. A fast "is my show right, and what does it look like on paper?"
//! check; pairs with the parse diagnostics (unknown transitions/heads warn on stderr).

use crate::scene::compose::Prop;
use crate::scene::sequence::{Sequence, shot_starts, show_end};
use crate::score::Score;
use crate::waypoints::{Waypoints, is_track};

/// The production kind a show declares (`kind = intro|demo` in `[settings]` → `MARTIN_KIND`). An
/// **intro** is a self-contained, asset-budgeted showcase that must bundle into the single binary; a
/// **demo** is the full-fat production (heavy local captures allowed). See DOMAIN.md §8 / L1.
pub enum Kind {
    Intro,
    Demo,
}

pub fn production_kind() -> Option<Kind> {
    match std::env::var("MARTIN_KIND")
        .ok()?
        .to_ascii_lowercase()
        .as_str()
    {
        "intro" => Some(Kind::Intro),
        "demo" => Some(Kind::Demo),
        other => {
            eprintln!("MARTIN_KIND: unknown kind '{other}' (expected intro|demo) — ignored");
            None
        }
    }
}

/// A heavy single asset (bytes) bloats an intro's bundle; flag anything over this.
const INTRO_HEAVY_ASSET: u64 = 8 * 1024 * 1024;

/// Print the parsed show to stdout. Called from `main` before the app is built when MARTIN_VALIDATE
/// is set; `main` then returns (no window, no render). `asset_root` resolves the referenced files for
/// the `intro` budget check.
pub fn report(
    seq: &Sequence,
    compose: &[Prop],
    cam: &Waypoints,
    score: &Score,
    asset_root: Option<&str>,
) {
    println!("\nmartin — show validation (dry run, no render)\n");
    if let Some(kind) = production_kind() {
        let name = match kind {
            Kind::Intro => "intro (self-contained, asset-budgeted, bundles)",
            Kind::Demo => "demo (full-fat; local captures allowed)",
        };
        println!("production: {name}");
        if matches!(kind, Kind::Intro) {
            intro_budget(seq, compose, asset_root.unwrap_or("assets"));
        }
    }
    println!(
        "score:    {} sections, ~{:.0}s",
        score.sections.len(),
        score.demo_len()
    );

    if !seq.parts.is_empty() {
        let starts = shot_starts(&seq.parts);
        let end = show_end(&seq.parts, &starts);
        println!(
            "\nsequence: {} parts, ~{:.0}s total, {} gaussians/part",
            seq.parts.len(),
            end,
            seq.budget
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
        for (i, w) in cam.list.iter().enumerate() {
            let t = w.t.map(|t| format!("t={t:6.1}s  ")).unwrap_or_default();
            // annotate each Key with the CameraMove of the segment that arrives at it (from the
            // previous Key) — so the dry-run reads back what the keyframes actually do.
            let mv = i
                .checked_sub(1)
                .map(|p| {
                    format!(
                        "  [{}]",
                        crate::waypoints::CameraMove::infer(&cam.list[p], w).label()
                    )
                })
                .unwrap_or_default();
            println!(
                "  {t}dist {:.2}  yaw {:.2}  pitch {:.2}  target ({:.1},{:.1},{:.1}){mv}",
                w.dist, w.yaw, w.pitch, w.target.x, w.target.y, w.target.z,
            );
        }
    }
    println!();
}

/// The `intro` self-containment check: an intro must bundle into one binary, so every asset it
/// references should be present + light + repo-shippable. Sums the referenced asset bytes and warns
/// on the three ways an intro breaks that promise: a **missing** file, a **heavy** one (bloats the
/// bundle), or a **local capture** (not self-contained). Procedural shapes (synthesized by build.rs)
/// count toward the budget but never warn as "missing" — they're regenerated.
fn intro_budget(seq: &Sequence, compose: &[Prop], root: &str) {
    use std::collections::BTreeSet;
    let root = std::path::Path::new(root);
    let mut names: BTreeSet<&str> = BTreeSet::new();
    for p in &seq.parts {
        names.extend(p.content.asset_files());
    }
    for o in compose {
        names.extend(o.content().asset_files());
    }
    let (mut total, mut counted) = (0u64, 0usize);
    for name in &names {
        let path = root.join(name);
        match std::fs::metadata(&path) {
            Ok(m) => {
                total += m.len();
                counted += 1;
                if m.len() > INTRO_HEAVY_ASSET {
                    eprintln!(
                        "intro: heavy asset '{name}' ({:.1} MB) — bloats the bundle, consider downsampling",
                        m.len() as f64 / 1.048_576e6
                    );
                }
            }
            // build.rs synthesizes the procedural shapes on build, so absence isn't fatal for those.
            Err(_) => eprintln!(
                "intro: '{name}' not present (a capture or un-generated shape) — an intro must be self-contained"
            ),
        }
        if name.contains("captures/") || name.contains("/captures") {
            eprintln!("intro: '{name}' is a local capture — won't ship in a clean checkout");
        }
    }
    println!(
        "intro budget: {:.1} MB across {counted} present asset(s) (of {} referenced)",
        total as f64 / 1.048_576e6,
        names.len()
    );
}
