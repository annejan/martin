//! Parse-time logic: build the `Sequence` from `MARTIN_SEQ` / `MARTIN_TEXT` / `MARTIN_PLY`, plus
//! the small parsers it leans on (`raster:`, `rot:` euler degrees) and the capture-camera loader.

use bevy::prelude::*;
use bevy_gaussian_splatting::RasterizeMode;

use super::model::{Sequence, Shot};
use crate::scene::content::{PartContent, parse_source, side_by_side};
use crate::scene::effects::{Deform, Departure, Transition};
use crate::scene::{file_name_of, parent_dir};
use crate::score;

/// `raster:<mode>` / `MARTIN_RASTER=<mode>` → the fork's RasterizeMode (the debug-shading views from
/// the upstream viewer: colour is the normal render, the rest visualise a channel). None on a bad name.
pub(crate) fn parse_raster(s: &str) -> Option<RasterizeMode> {
    Some(match s.trim().to_ascii_lowercase().as_str() {
        "color" | "colour" | "rgb" => RasterizeMode::Color,
        "depth" => RasterizeMode::Depth,
        "normal" | "normals" => RasterizeMode::Normal,
        "position" | "pos" => RasterizeMode::Position,
        "classification" | "class" => RasterizeMode::Classification,
        "opticalflow" | "optical-flow" | "flow" => RasterizeMode::OpticalFlow,
        "velocity" | "vel" => RasterizeMode::Velocity,
        _ => return None,
    })
}

/// The global default raster mode (`MARTIN_RASTER`), applied to any part without its own `raster:`.
pub(crate) fn global_raster() -> RasterizeMode {
    std::env::var("MARTIN_RASTER")
        .ok()
        .and_then(|s| {
            let m = parse_raster(&s);
            if m.is_none() {
                eprintln!("MARTIN_RASTER: unknown mode '{s}' — using color");
            }
            m
        })
        .unwrap_or(RasterizeMode::Color)
}

/// Load the capture-camera world positions from a 3DGS/COLMAP `cameras.json` (graphdeco format:
/// an array of objects each with a `"position": [x,y,z]`). These are in the same coordinates as
/// the scene's `.ply`, so applying the scene's normalize + cloud rotation places martin's camera
/// where the scene was actually shot — the only viewpoint a 360° capture renders coherently.
pub(crate) fn load_camera_positions(path: &str) -> Vec<Vec3> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    json.as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|c| {
                    let p = c.get("position")?.as_array()?;
                    Some(Vec3::new(
                        p.first()?.as_f64()? as f32,
                        p.get(1)?.as_f64()? as f32,
                        p.get(2)?.as_f64()? as f32,
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Parse `MARTIN_SEQ`: a file path OR an inline string. Parts are `;`/newline-separated.
/// Each part: `text:STRING` or `splat:a.ply` (or `a.ply+b.ply` for side-by-side), optional
/// trailing `@hold,morph,bulge`. `#` comments and blank lines are skipped.
pub(crate) fn parse_seq(spec: &str, score: &score::Score) -> Vec<Shot> {
    let raw = std::fs::read_to_string(spec).unwrap_or_else(|_| spec.to_string());
    let mut parts = Vec::new();
    // strip each line's `#` comment to end-of-line FIRST (so a `;` inside a comment can't split it
    // and leak the tail as a bogus part), then split into parts on `;`/newline.
    let cleaned: String = raw
        .lines()
        .map(|l| l.split('#').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    for line in cleaned.split([';', '\n']) {
        let s = line.trim();
        if s.is_empty() {
            continue;
        }
        // Pull the `~name` transition AND the `@@anchor` token (both single whitespace-delimited
        // tokens, position-independent); keep the rest of the line for the head + `@timing`.
        let mut transition = None;
        let mut anchor = None;
        let mut deform = None;
        let mut out = None;
        let mut rot = None;
        let mut cluster = None;
        let mut bg = None;
        let mut raster = None;
        let mut flash = None;
        let mut deform_amp = None;
        let mut beat = None;
        // Pull each modifier token out of the line by its sigil/prefix. A token carrying a known
        // prefix is ALWAYS consumed (never leaks into the head/text) — if it fails to parse we warn,
        // so a typo (`~explod`, `^wave2`) is a visible error, not a silently-dropped effect.
        let s: String = s
            .split_whitespace()
            .filter(|tok| {
                if let Some(a) = tok.strip_prefix("@@") {
                    match score.anchor_seconds(a) {
                        Some(sec) => anchor = Some(sec),
                        None => {
                            eprintln!("seq: unknown anchor '@@{a}' (no such section/cue) — ignored")
                        }
                    }
                } else if let Some(d) = tok.strip_prefix("exit:").or(tok.strip_prefix("out:")) {
                    match Departure::parse(d) {
                        Some(dep) => out = Some(dep),
                        None => eprintln!("seq: unknown exit 'exit:{d}' — ignored"),
                    }
                } else if let Some(r) = tok.strip_prefix("rot:") {
                    match parse_euler_deg(r) {
                        Some(q) => rot = Some(q),
                        None => eprintln!("seq: bad 'rot:{r}' (need rx,ry,rz degrees) — ignored"),
                    }
                } else if let Some(c) = tok.strip_prefix("flock:").or(tok.strip_prefix("cluster:"))
                {
                    match c.parse() {
                        Ok(n) => cluster = Some(n),
                        Err(_) => eprintln!("seq: bad 'flock:{c}' (need an integer) — ignored"),
                    }
                } else if let Some(b) = tok.strip_prefix("backdrop:").or(tok.strip_prefix("bg:")) {
                    bg = Some(crate::background::bg_token(b)); // warns + falls back inside
                } else if let Some(r) = tok.strip_prefix("raster:") {
                    match parse_raster(r) {
                        Some(m) => raster = Some(m),
                        None => eprintln!("seq: unknown raster mode 'raster:{r}' — ignored"),
                    }
                } else if let Some(f) = tok.strip_prefix("flash:") {
                    match f.parse() {
                        Ok(v) => flash = Some(v),
                        Err(_) => eprintln!("seq: bad 'flash:{f}' (need a number) — ignored"),
                    }
                } else if let Some(b) = tok.strip_prefix("beat:") {
                    match b.parse() {
                        Ok(v) => beat = Some(v),
                        Err(_) => eprintln!("seq: bad 'beat:{b}' (need a number) — ignored"),
                    }
                } else if let Some(d) = tok.strip_prefix('^') {
                    // `^name` or `^name:amp` — the optional amp scales this shot's deform strength.
                    let (name, amp) = d.split_once(':').map_or((d, None), |(n, a)| (n, Some(a)));
                    match Deform::parse(name) {
                        Some(de) => {
                            deform = Some(de);
                            if let Some(a) = amp {
                                match a.parse() {
                                    Ok(v) => deform_amp = Some(v),
                                    Err(_) => eprintln!("seq: bad deform amp '^{d}' — using 1.0"),
                                }
                            }
                        }
                        None => eprintln!("seq: unknown deform '^{d}' — ignored"),
                    }
                } else if let Some(t) = tok.strip_prefix('~') {
                    match Transition::parse(t) {
                        Some(tr) => transition = Some(tr),
                        None => eprintln!("seq: unknown transition '~{t}' — ignored"),
                    }
                } else {
                    return true; // not a modifier → keep it for the head + @timing
                }
                false // a modifier token → consume it
            })
            .collect::<Vec<_>>()
            .join(" ");
        let (head, timing) = match s.split_once('@') {
            Some((h, t)) => (h.trim(), Some(t.trim())),
            None => (s.as_str(), None),
        };
        let (mut hold, mut morph, mut bulge) = (1.5_f32, 3.0_f32, 0.9_f32);
        if let Some(t) = timing {
            let nums: Vec<f32> = t.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if let Some(v) = nums.first() {
                hold = *v;
            }
            if let Some(v) = nums.get(1) {
                morph = *v;
            }
            if let Some(v) = nums.get(2) {
                bulge = *v;
            }
        }
        let Some(content) = parse_source(head) else {
            eprintln!(
                "seq: unrecognized part '{head}' — expected one of \
                 text:/svg:/image:/mesh:/glb:/shader:/splat:/wall: — skipped"
            );
            continue;
        };
        parts.push(Shot {
            content,
            hold,
            morph,
            bulge,
            transition,
            anchor,
            deform,
            out,
            rot,
            cluster,
            bg,
            raster,
            flash,
            deform_amp,
            beat,
        });
    }
    parts
}

/// Parse `rx,ry,rz` euler **degrees** into a quaternion (for a part's `rot:` token). Needs all three.
pub(crate) fn parse_euler_deg(s: &str) -> Option<Quat> {
    let n: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    (n.len() == 3).then(|| {
        Quat::from_euler(
            EulerRot::XYZ,
            n[0].to_radians(),
            n[1].to_radians(),
            n[2].to_radians(),
        )
    })
}

/// Build the show: `MARTIN_SEQ` if set, else a shorthand from `MARTIN_TEXT` /
/// `MARTIN_PLY(+_PLY2)(+_REFORM)`. Returns the sequence + the asset root (the .ply folder).
pub(crate) fn sequence_from_env(score: &score::Score) -> (Sequence, Option<String>) {
    // The default demo is now `assets/demo.show` (set as MARTIN_SHOW in `main` when nothing is
    // requested), so by the time we get here MARTIN_SEQ is set from its `[seq]` section.
    let budget_default = if std::env::var("MARTIN_SEQ").is_ok() {
        200_000
    } else {
        0
    };
    let budget = crate::envvar::or("MARTIN_MORPH_COUNT", budget_default);

    if let Ok(spec) = std::env::var("MARTIN_SEQ") {
        // asset root = the .ply folder (so `splat:` filenames resolve); MARTIN_PLY sets it.
        let root = std::env::var("MARTIN_PLY").ok().and_then(parent_dir);
        return (
            Sequence {
                parts: parse_seq(&spec, score),
                budget,
            },
            root,
        );
    }

    if let Ok(text) = std::env::var("MARTIN_TEXT") {
        let part = Shot {
            content: PartContent::Text(text),
            hold: 2.0,
            morph: 3.0,
            bulge: 0.0,
            transition: None,
            anchor: None,
            deform: None,
            out: None,
            rot: None,
            cluster: None,
            bg: None,
            raster: None,
            flash: None,
            deform_amp: None,
            beat: None,
        };
        return (
            Sequence {
                parts: vec![part],
                budget,
            },
            None,
        );
    }

    // splat shorthand: PLY (+ PLY2) as part 0; REFORM (if any) as part 1.
    let primary = std::env::var("MARTIN_PLY").ok();
    let root = primary.as_deref().and_then(|p| parent_dir(p.to_string()));
    let name1 = primary
        .as_deref()
        .map(file_name_of)
        .unwrap_or_else(|| "aegg.ply".into());
    let mut names = vec![name1];
    if let Ok(p2) = std::env::var("MARTIN_PLY2") {
        names.push(file_name_of(&p2));
    }
    let bulge = crate::envvar::or("MARTIN_BULGE", 0.9);
    let mut parts = vec![Shot {
        content: PartContent::Splats(side_by_side(names.iter().map(String::as_str))),
        hold: 2.0,
        morph: 3.0,
        bulge: 0.0,
        transition: None,
        anchor: None,
        deform: None,
        out: None,
        rot: None,
        cluster: None,
        bg: None,
        raster: None,
        flash: None,
        deform_amp: None,
        beat: None,
    }];
    if let Ok(reform) = std::env::var("MARTIN_REFORM") {
        parts.push(Shot {
            content: PartContent::Splats(vec![(file_name_of(&reform), Vec3::ZERO)]),
            hold: 2.0,
            morph: 3.5,
            bulge,
            transition: None,
            anchor: None,
            deform: None,
            out: None,
            rot: None,
            cluster: None,
            bg: None,
            raster: None,
            flash: None,
            deform_amp: None,
            beat: None,
        });
    }
    (Sequence { parts, budget }, root)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::sequence::{shot_starts, show_end};

    fn parts(spec: &str) -> Vec<Shot> {
        parse_seq(spec, &score::Score::builtin())
    }

    #[test]
    fn parse_seq_reads_heads_timing_and_modifiers() {
        let p = parts("text:HELLO @4,2 ~fade ^wave out:sink rot:0,90,0 cluster:3");
        assert_eq!(p.len(), 1);
        assert!(matches!(&p[0].content, PartContent::Text(s) if s == "HELLO"));
        assert_eq!(p[0].hold, 4.0);
        assert_eq!(p[0].morph, 2.0);
        assert_eq!(p[0].transition, Some(Transition::Fade));
        assert_eq!(p[0].deform, Some(Deform::Wave));
        assert_eq!(p[0].out, Some(Departure::Sink));
        assert_eq!(p[0].cluster, Some(3));
        assert!(p[0].rot.is_some());
    }

    #[test]
    fn parse_seq_reads_raster_token() {
        let p = parts("text:HI ~outline raster:position");
        assert_eq!(p[0].raster, Some(RasterizeMode::Position));
        assert_eq!(parse_raster("normals"), Some(RasterizeMode::Normal)); // alias
        assert_eq!(parse_raster("DEPTH"), Some(RasterizeMode::Depth)); // case
        assert_eq!(parse_raster("nope"), None);
        // a bad raster: token leaves the part with no override (falls back to MARTIN_RASTER/color)
        assert_eq!(parts("text:HI raster:bogus")[0].raster, None);
    }

    #[test]
    fn parse_seq_splits_parts_and_skips_unknown_heads() {
        // `txet:` is a typo → that part is skipped (warned), the others survive.
        let p = parts("text:A; txet:B; text:C");
        assert_eq!(p.len(), 2);
        assert!(matches!(&p[0].content, PartContent::Text(s) if s == "A"));
        assert!(matches!(&p[1].content, PartContent::Text(s) if s == "C"));
    }

    #[test]
    fn unknown_modifier_is_consumed_not_leaked_into_the_head() {
        // a typo'd transition must NOT end up as part of the text.
        let p = parts("text:HELLO ~explod");
        assert_eq!(p.len(), 1);
        assert!(matches!(&p[0].content, PartContent::Text(s) if s == "HELLO"));
        assert_eq!(p[0].transition, None);
    }

    #[test]
    fn comment_with_a_semicolon_does_not_resurrect_a_bogus_part() {
        // regression: a `;` inside a `#` comment used to split it and parse the tail as a part.
        let p = parts("text:A   # note; with a ~semicolon and ~fade inside\ntext:B");
        assert_eq!(p.len(), 2);
        assert!(matches!(&p[0].content, PartContent::Text(s) if s == "A"));
        assert!(matches!(&p[1].content, PartContent::Text(s) if s == "B"));
        assert_eq!(p[0].transition, None); // the ~fade was inside the comment
    }

    #[test]
    fn shot_starts_lay_end_to_end_then_honour_anchors() {
        let p = parts("text:A @2,1; text:B @3,1; text:C @1,1");
        let s = shot_starts(&p);
        assert_eq!(s[0], 0.0);
        assert_eq!(s[1], 3.0); // 0 + morph 1 + hold 2
        assert_eq!(s[2], 7.0); // 3 + 1 + 3
        assert_eq!(show_end(&p, &s), 9.0); // 7 + 1 + 1
    }

    #[test]
    fn active_shot_picks_the_latest_started() {
        let starts = [0.0, 3.0, 7.0];
        assert_eq!(crate::scene::sequence::active_shot(&starts, 0.0), 0);
        assert_eq!(crate::scene::sequence::active_shot(&starts, 2.9), 0);
        assert_eq!(crate::scene::sequence::active_shot(&starts, 3.0), 1);
        assert_eq!(crate::scene::sequence::active_shot(&starts, 100.0), 2);
    }

    #[test]
    fn parse_euler_deg_needs_three_components() {
        assert!(parse_euler_deg("0,90,0").is_some());
        assert!(parse_euler_deg("0,90").is_none());
        assert!(parse_euler_deg("x,y,z").is_none());
    }
}
