//! `[sync]` — a music-timed **look track**, the GNU-Rocket-style automation layer (DOMAIN.md §9
//! Stage 4 / §6 SyncTrack). Where `[camera]` keyframes the camera pose over the score clock, `[sync]`
//! keyframes the *global look knobs* — `flash`, `bg_dim`, `beat` — so the bloom can swell into the
//! drop, the backdrop dim through the breakdown, the beat-reactivity ramp to the climax, etc.,
//! interpolated (smoothstep) between keyframes instead of one static value for the whole show.
//!
//! Syntax (same grammar as `[camera]`): one keyframe per line, many knobs per keyframe:
//! ```text
//! [sync]
//! t=@@intro     flash=0.0  bg_dim=0.5  beat=0.4
//! t=@@drop      flash=0.6  bg_dim=0.3  beat=1.3
//! t=@@climax    flash=0.8  bg_dim=0.15 beat=1.6
//! ```
//! A knob omitted on a line is simply not a keyframe for it (each knob has its own keyframe list).
//! No `[sync]` section (or a knob with no keyframes) → the static `MARTIN_*` value is used, unchanged.

use bevy::prelude::*;

/// Per-knob keyframe lists `(time_s, value)`, each sorted by time. Resolved per frame by the consumer
/// systems (`shot_director` for `flash`, `update_beat` for `beat`, `update_bg` for `bg_dim`) — each
/// reads its own knob at its own point, so there's no cross-system ordering to get wrong.
#[derive(Resource, Default)]
pub struct SyncTrack {
    flash: Vec<(f32, f32)>,
    bg_dim: Vec<(f32, f32)>,
    beat: Vec<(f32, f32)>,
}

impl SyncTrack {
    pub fn flash_at(&self, t: f32) -> Option<f32> {
        eval(&self.flash, t)
    }
    pub fn bg_dim_at(&self, t: f32) -> Option<f32> {
        eval(&self.bg_dim, t)
    }
    pub fn beat_at(&self, t: f32) -> Option<f32> {
        eval(&self.beat, t)
    }
    pub fn is_empty(&self) -> bool {
        self.flash.is_empty() && self.bg_dim.is_empty() && self.beat.is_empty()
    }

    /// `(knob, keyframes)` for each knob that has any — for the `MARTIN_VALIDATE` dump.
    pub fn knobs(&self) -> Vec<(&'static str, &[(f32, f32)])> {
        [
            ("flash", &self.flash),
            ("bg_dim", &self.bg_dim),
            ("beat", &self.beat),
        ]
        .into_iter()
        .filter(|(_, k)| !k.is_empty())
        .map(|(n, k)| (n, k.as_slice()))
        .collect()
    }
}

/// Evaluate a keyframe list at time `t`: clamp before the first / after the last, smoothstep between.
fn eval(keys: &[(f32, f32)], t: f32) -> Option<f32> {
    match keys {
        [] => None,
        [(_, v)] => Some(*v),
        _ => {
            if t <= keys[0].0 {
                return Some(keys[0].1);
            }
            if t >= keys[keys.len() - 1].0 {
                return Some(keys[keys.len() - 1].1);
            }
            let i = keys.partition_point(|&(kt, _)| kt <= t).max(1);
            let (t0, v0) = keys[i - 1];
            let (t1, v1) = keys[i];
            let f = ((t - t0) / (t1 - t0).max(1e-6)).clamp(0.0, 1.0);
            let s = f * f * (3.0 - 2.0 * f); // smoothstep
            Some(v0 + (v1 - v0) * s)
        }
    }
}

/// Parse `[sync]` lines (same time grammar as `[camera]`): `t=<secs|@@anchor> knob=value …`. The
/// score resolves the anchors. Unknown knobs warn and are skipped.
pub fn parse_sync(lines: &[String], score: &crate::score::Score) -> SyncTrack {
    let mut track = SyncTrack::default();
    for line in lines {
        let s = line.split('#').next().unwrap_or("").trim();
        if s.is_empty() {
            continue;
        }
        // resolve the keyframe time first (the `t=`/`time=` token).
        let time = s.split_whitespace().find_map(|tok| {
            let (k, v) = tok.split_once('=')?;
            if k != "t" && k != "time" {
                return None;
            }
            match v.strip_prefix("@@") {
                Some(a) => score.anchor_seconds(a),
                None => v.parse().ok(),
            }
        });
        let Some(time) = time else {
            eprintln!("sync: line without a valid `t=` time — skipped: {s}");
            continue;
        };
        for (k, v) in s.split_whitespace().filter_map(|t| t.split_once('=')) {
            let Ok(val) = v.parse::<f32>() else { continue };
            match k {
                "t" | "time" => {}
                "flash" => track.flash.push((time, val)),
                "bg_dim" | "dim" => track.bg_dim.push((time, val)),
                "beat" => track.beat.push((time, val)),
                other => {
                    eprintln!("sync: unknown knob '{other}' — skipped (have: flash/bg_dim/beat)")
                }
            }
        }
    }
    for v in [&mut track.flash, &mut track.bg_dim, &mut track.beat] {
        v.sort_by(|a, b| a.0.total_cmp(&b.0));
    }
    track
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eval_clamps_and_interpolates() {
        let k = vec![(0.0, 0.0), (10.0, 1.0)];
        assert_eq!(eval(&k, -1.0), Some(0.0)); // before → first
        assert_eq!(eval(&k, 11.0), Some(1.0)); // after → last
        assert_eq!(eval(&k, 5.0), Some(0.5)); // midpoint, smoothstep(0.5)=0.5
        assert_eq!(eval(&[], 3.0), None); // no keys → fall back to static
    }

    #[test]
    fn parse_resolves_time_and_knobs() {
        let score = crate::score::Score::builtin();
        let lines = vec![
            "t=0 flash=0.1 bg_dim=0.5".to_string(),
            "t=10 flash=0.9 beat=1.5".to_string(),
        ];
        let tr = parse_sync(&lines, &score);
        assert_eq!(tr.flash_at(0.0), Some(0.1));
        assert_eq!(tr.flash_at(10.0), Some(0.9));
        assert_eq!(tr.beat_at(10.0), Some(1.5));
        assert_eq!(tr.bg_dim_at(0.0), Some(0.5));
        assert!(tr.bg_dim_at(100.0).is_some()); // clamps to last bg_dim keyframe
    }
}
