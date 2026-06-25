//! The CLI front: `martin <show> [flags]` and `martin mcp`.
//!
//! clap compiles run-mode + utility flags into the SAME `MARTIN_*` env vars the `.show` file + the
//! bundle expand into — but with **overwrite** semantics, so a flag beats both an existing env var and
//! the `.show`. The precedence rule (**CLI flag > env > `.show` [settings] > built-in default**) falls
//! out of overwrite (flags) vs set-if-absent (`.show`/bundle/defaults) — no precedence logic is
//! duplicated. [`apply_cli`] is PURE (returns the `(var, value)` pairs, mutates nothing) so the
//! flag→env mapping is unit-tested without touching the process environment.

use clap::{Parser, Subcommand};

/// `martin [SHOW] [flags]` — the run modes (record / shot / bench / validate / serve / synth-wav /
/// dump-score) all compile to a `MARTIN_*` env var; the show's *look* stays in the `.show` file.
#[derive(Parser, Debug, Default)]
#[command(
    name = "martin",
    about = "fly a camera around morphing Gaussian splats",
    version
)]
pub struct Cli {
    /// A `.show` file to play (else `$MARTIN_SHOW`, else the bundled intro).
    pub show: Option<String>,
    /// Shorthand for `productions/<NAME>/<NAME>.show`.
    #[arg(long, value_name = "NAME")]
    pub production: Option<String>,
    /// Record the whole timeline into a frame directory (headless). Used by `record.sh`.
    #[arg(long, value_name = "DIR")]
    pub record: Option<String>,
    /// One headless screenshot to this path, then exit.
    #[arg(long, value_name = "PATH")]
    pub shot: Option<String>,
    /// When (seconds) to take `--shot`.
    #[arg(long = "shot-at", value_name = "SECS")]
    pub shot_at: Option<f32>,
    /// Contact sheet: comma-separated times (uses `--shot` as the path stem).
    #[arg(long, value_name = "T1,T2,…")]
    pub shots: Option<String>,
    /// Render N frames headless (no PNG), log the render-only fps, then exit.
    #[arg(long, value_name = "FRAMES")]
    pub bench: Option<u32>,
    /// Print the resolved timeline + exit (no render).
    #[arg(long)]
    pub validate: bool,
    /// Make a score phase/bar typo fatal.
    #[arg(long)]
    pub strict: bool,
    /// Run the HTTP control bridge (optional port, default 7878).
    #[arg(long, value_name = "PORT")]
    pub serve: Option<Option<u16>>,
    /// Render the synth to a WAV and exit.
    #[arg(long = "synth-wav", value_name = "PATH")]
    pub synth_wav: Option<String>,
    /// Write the built-in score as an editable tracker file and exit.
    #[arg(long = "dump-score", value_name = "PATH")]
    pub dump_score: Option<String>,
    /// Start borderless-fullscreen (overrides the build default; F11/F still toggle live).
    #[arg(long, conflicts_with = "windowed")]
    pub fullscreen: bool,
    /// Start in a window (overrides a fullscreen build default / `[settings] fullscreen`).
    #[arg(long)]
    pub windowed: bool,
    #[command(subcommand)]
    pub command: Option<Commands>,
}

/// Subcommands (none required — a bare `martin [SHOW]` runs the show).
#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run the stdio MCP server (a proxy to a `--serve` bridge), then exit — no Bevy, so stdout stays
    /// clean JSON-RPC. `.mcp.json` invokes `martin mcp`.
    Mcp {
        /// Bridge port to proxy to (else `$MARTIN_MCP_PORT` / `$MARTIN_SERVE` / 7878).
        #[arg(long, value_name = "PORT")]
        port: Option<u16>,
    },
}

/// The `MARTIN_*` env vars a CLI invocation sets, as ordered `(var, value)` pairs — PURE (no env
/// mutation). `main` applies these with `set_var` (overwrite) BEFORE the `.show`/bundle expansion, so a
/// flag wins over both. Order matters: `--production` is pushed before the positional `SHOW`, so the
/// positional wins when both are given (the later `set_var` overwrites).
pub fn apply_cli(cli: &Cli) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    if let Some(name) = &cli.production {
        out.push((
            "MARTIN_SHOW".into(),
            format!("productions/{name}/{name}.show"),
        ));
    }
    if let Some(s) = &cli.show {
        out.push(("MARTIN_SHOW".into(), s.clone()));
    }
    if let Some(d) = &cli.record {
        out.push(("MARTIN_RECORD".into(), d.clone()));
    }
    if let Some(p) = &cli.shot {
        out.push(("MARTIN_SHOT".into(), p.clone()));
    }
    if let Some(t) = cli.shot_at {
        out.push(("MARTIN_SHOT_AT".into(), t.to_string()));
    }
    if let Some(ts) = &cli.shots {
        out.push(("MARTIN_SHOTS".into(), ts.clone()));
    }
    if let Some(n) = cli.bench {
        out.push(("MARTIN_BENCH".into(), n.to_string()));
    }
    if cli.validate {
        out.push(("MARTIN_VALIDATE".into(), "1".into()));
    }
    if cli.strict {
        out.push(("MARTIN_SCORE_STRICT".into(), "1".into()));
    }
    if let Some(port) = cli.serve {
        // presence enables the bridge; a given port also sets it (serve.rs reads MARTIN_SERVE).
        out.push((
            "MARTIN_SERVE".into(),
            port.map(|p| p.to_string()).unwrap_or_else(|| "1".into()),
        ));
    }
    if let Some(p) = &cli.synth_wav {
        out.push(("MARTIN_SYNTH_WAV".into(), p.clone()));
    }
    if let Some(p) = &cli.dump_score {
        out.push(("MARTIN_SCORE_DUMP".into(), p.clone()));
    }
    // Window mode override: --fullscreen/--windowed → MARTIN_FULLSCREEN=1/0 (tri-state — unset falls
    // through to the build default; see main.rs). clap makes the two flags mutually exclusive.
    if cli.fullscreen {
        out.push(("MARTIN_FULLSCREEN".into(), "1".into()));
    } else if cli.windowed {
        out.push(("MARTIN_FULLSCREEN".into(), "0".into()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find<'a>(pairs: &'a [(String, String)], key: &str) -> Option<&'a str> {
        // last write wins (mirrors how main applies them with overwrite set_var)
        pairs
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    #[test]
    fn run_mode_flags_map_to_env() {
        let cli = Cli {
            record: Some("/tmp/frames".into()),
            shot: Some("/tmp/x.png".into()),
            shot_at: Some(12.5),
            shots: Some("3,18".into()),
            bench: Some(120),
            validate: true,
            strict: true,
            synth_wav: Some("/tmp/a.wav".into()),
            dump_score: Some("/tmp/s.txt".into()),
            ..Default::default()
        };
        let p = apply_cli(&cli);
        assert_eq!(find(&p, "MARTIN_RECORD"), Some("/tmp/frames"));
        assert_eq!(find(&p, "MARTIN_SHOT"), Some("/tmp/x.png"));
        assert_eq!(find(&p, "MARTIN_SHOT_AT"), Some("12.5"));
        assert_eq!(find(&p, "MARTIN_SHOTS"), Some("3,18"));
        assert_eq!(find(&p, "MARTIN_BENCH"), Some("120"));
        assert_eq!(find(&p, "MARTIN_VALIDATE"), Some("1"));
        assert_eq!(find(&p, "MARTIN_SCORE_STRICT"), Some("1"));
        assert_eq!(find(&p, "MARTIN_SYNTH_WAV"), Some("/tmp/a.wav"));
        assert_eq!(find(&p, "MARTIN_SCORE_DUMP"), Some("/tmp/s.txt"));
    }

    #[test]
    fn nothing_set_is_empty() {
        assert!(apply_cli(&Cli::default()).is_empty());
    }

    #[test]
    fn window_mode_flags_map_to_fullscreen_env() {
        let fs = apply_cli(&Cli {
            fullscreen: true,
            ..Default::default()
        });
        assert_eq!(find(&fs, "MARTIN_FULLSCREEN"), Some("1"));
        let win = apply_cli(&Cli {
            windowed: true,
            ..Default::default()
        });
        assert_eq!(find(&win, "MARTIN_FULLSCREEN"), Some("0"));
        // neither flag → no override (falls through to the build default in main)
        assert_eq!(find(&apply_cli(&Cli::default()), "MARTIN_FULLSCREEN"), None);
    }

    #[test]
    fn production_resolves_to_show_path() {
        let cli = Cli {
            production: Some("guinea".into()),
            ..Default::default()
        };
        assert_eq!(
            find(&apply_cli(&cli), "MARTIN_SHOW"),
            Some("productions/guinea/guinea.show")
        );
    }

    #[test]
    fn positional_show_wins_over_production() {
        let cli = Cli {
            production: Some("guinea".into()),
            show: Some("other.show".into()),
            ..Default::default()
        };
        // both push MARTIN_SHOW; the positional is pushed last → overwrite wins
        assert_eq!(find(&apply_cli(&cli), "MARTIN_SHOW"), Some("other.show"));
    }

    #[test]
    fn serve_default_and_explicit_port() {
        assert_eq!(
            find(
                &apply_cli(&Cli {
                    serve: Some(None),
                    ..Default::default()
                }),
                "MARTIN_SERVE"
            ),
            Some("1")
        );
        assert_eq!(
            find(
                &apply_cli(&Cli {
                    serve: Some(Some(9090)),
                    ..Default::default()
                }),
                "MARTIN_SERVE"
            ),
            Some("9090")
        );
    }
}
