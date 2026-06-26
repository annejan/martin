// SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
// SPDX-License-Identifier: MIT

//! `--benchmark`: auto-tune for the machine the binary lands on. The PARENT re-launches itself once
//! per quality tier; each child runs the *actual* show **windowed** (pinned to one moment, vsync off)
//! with that tier's caps, logging the engine's own fps metric — exactly the method
//! `pipeline/bench-sweep.sh` uses, just in-process. The parent reads each child's steady-state fps,
//! prints a table, and recommends the best tier that clears the target. Windowed (not headless) on
//! purpose: the live demo pipelines a render thread alongside the main schedule, so a headless
//! `ScheduleRunner` would measure a CPU-schedule-bound rate that doesn't reflect the real experience.

use bevy::prelude::*;

/// Tiers swept, ascending quality — the recommendation picks the HIGHEST that clears the target.
const TIERS: &[&str] = &["potato", "low", "med", "high"];
/// Seconds each child renders before exiting — long enough for the build + a few steady fps samples.
const RUN_SECONDS: f32 = 7.0;
/// Leading fps samples dropped (loader + off-thread build + pipeline settle) before the median.
const DROP_SAMPLES: usize = 6;

/// True when this process is the benchmark PARENT (run the orchestrator, no Bevy app).
pub fn is_parent() -> bool {
    std::env::var_os("MARTIN_BENCHMARK").is_some()
        && std::env::var_os("MARTIN_BENCHMARK_CHILD").is_none()
}

/// True when this process is a benchmark CHILD (a normal windowed run that auto-exits + is fps-logged).
pub fn is_child() -> bool {
    std::env::var_os("MARTIN_BENCHMARK_CHILD").is_some()
}

/// Parent: spawn a child per tier, read its steady-state fps, print the table + the recommendation.
pub fn run_parent() {
    let at = crate::envvar::or("MARTIN_BENCHMARK_AT", 30.0_f32);
    let target = crate::envvar::or("MARTIN_BENCHMARK_TARGET", 30.0_f32);
    let Ok(exe) = std::env::current_exe() else {
        eprintln!("benchmark: can't find own executable — aborting");
        return;
    };
    eprintln!(
        "benchmark: measuring {} tiers at t={at}s, target {target:.0} fps (a window renders each, ~{RUN_SECONDS:.0}s)…",
        TIERS.len()
    );

    let mut rows: Vec<(&str, String, Option<f32>)> = Vec::new();
    for &tier in TIERS {
        let res = crate::quality_caps(tier)
            .iter()
            .find(|(k, _)| *k == "MARTIN_RES")
            .map(|(_, v)| v.to_string())
            .unwrap_or_else(|| "?".into());
        let out = std::process::Command::new(&exe)
            .env_remove("MARTIN_BENCHMARK")
            .env("MARTIN_BENCHMARK_CHILD", "1")
            .env("MARTIN_QUALITY", tier)
            .env("MARTIN_HOLD_T", at.to_string())
            .env("MARTIN_FPS", "1") // engine logs `metrics: <fps> fps` every ~0.5 s
            .env("MARTIN_VSYNC", "0") // uncapped present — the real GPU ceiling, not the refresh rate
            .env("MARTIN_MUTE", "1") // no synth render/playback while benchmarking
            .output();
        // The metric is logged to stderr (tracing). Take the steady-state samples (drop the build tail).
        let fps = out
            .ok()
            .and_then(|o| median(&steady_samples(&String::from_utf8_lossy(&o.stderr))));
        eprintln!(
            "  {tier:<7} {res:<11} {}",
            fps.map(|f| format!("{f:.1} fps"))
                .unwrap_or_else(|| "FAILED".into())
        );
        rows.push((tier, res, fps));
    }

    print_report(&rows, target);
}

/// Every `metrics: <fps> fps` value in a child's log, with the first `DROP_SAMPLES` (loader + build +
/// pipeline settle) dropped — the rest are the steady-state scene.
fn steady_samples(log: &str) -> Vec<f32> {
    let all: Vec<f32> = log
        .lines()
        .filter_map(|l| l.split_once("metrics:"))
        .filter_map(|(_, rest)| rest.split_whitespace().next())
        .filter_map(|n| n.parse().ok())
        .collect();
    if all.len() > DROP_SAMPLES {
        all[DROP_SAMPLES..].to_vec()
    } else {
        all // too few to drop a warm-up — use what we have
    }
}

/// Median of a sample set (None when empty).
fn median(xs: &[f32]) -> Option<f32> {
    if xs.is_empty() {
        return None;
    }
    let mut v = xs.to_vec();
    v.sort_by(f32::total_cmp);
    Some(v[v.len() / 2])
}

/// Pick the highest-quality tier that clears `target` (rows are ascending quality, so search reversed).
fn recommend<'a>(rows: &'a [(&'a str, String, Option<f32>)], target: f32) -> Option<&'a str> {
    rows.iter()
        .rev()
        .find(|(_, _, f)| f.is_some_and(|v| v >= target))
        .map(|(t, _, _)| *t)
}

fn print_report(rows: &[(&str, String, Option<f32>)], target: f32) {
    println!("\n=== martin benchmark — this GPU ===");
    for (tier, res, fps) in rows {
        let mark = match fps {
            Some(f) if *f >= target => "✓",
            Some(_) => " ",
            None => "✗",
        };
        let shown = fps
            .map(|f| format!("{f:.1} fps"))
            .unwrap_or_else(|| "—".into());
        println!("  {tier:<7} {res:<11} {shown:>9}  {mark}");
    }
    match recommend(rows, target) {
        Some(t) => {
            println!("\n→ for ≥{target:.0} fps run:  --quality {t}   (or MARTIN_QUALITY={t})")
        }
        None => println!(
            "\n→ no tier clears {target:.0} fps here. Try --benchmark-at a lighter moment, a lower \
             show budget, or accept the best above."
        ),
    }
    // Honest caveat: each tier is timed in a SPAWNED window, which most compositors throttle when it
    // isn't focused — so on an unfocused desktop / over SSH the numbers read low (~4× on the dev box).
    // A focused or fullscreen session (e.g. the shipped exe at a party) measures the true rate.
    println!(
        "  (note: needs a FOCUSED/fullscreen window to be accurate — an unfocused/SSH session reads low.)\n"
    );
}

/// Child plugin: render windowed (so the live render-thread pipeline is in play) and **exit after
/// `RUN_SECONDS`** so the parent can collect the `MARTIN_FPS` metrics it logged. Added only in children.
pub fn plugin(app: &mut App) {
    app.add_systems(Update, auto_exit);
}

fn auto_exit(time: Res<Time<Real>>, mut exit: MessageWriter<AppExit>) {
    if time.elapsed_secs() >= RUN_SECONDS {
        exit.write(AppExit::Success);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steady_samples_drops_the_warmup_tail() {
        // 8 samples → drop the first DROP_SAMPLES(6), keep the last 2 (the steady scene)
        let log = (0..8)
            .map(|i| format!("INFO metrics: {}.0 fps (x ms) t=1", 10 + i))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(steady_samples(&log), vec![16.0, 17.0]);
        // too few to trim → keep all
        assert_eq!(steady_samples("metrics: 42.0 fps"), vec![42.0]);
        assert!(steady_samples("no metrics here").is_empty());
    }

    #[test]
    fn median_of_samples() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[40.0, 10.0, 30.0]), Some(30.0));
    }

    #[test]
    fn recommend_picks_highest_tier_clearing_target() {
        let rows = vec![
            ("potato", "640x360".into(), Some(75.0)),
            ("low", "854x480".into(), Some(45.0)),
            ("med", "1280x720".into(), Some(30.5)),
            ("high", "1920x1080".into(), Some(14.0)),
        ];
        assert_eq!(recommend(&rows, 30.0), Some("med")); // highest ≥30
        assert_eq!(recommend(&rows, 60.0), Some("potato")); // only potato clears 60
        assert_eq!(recommend(&rows, 99.0), None); // nothing clears 99
    }
}
