//! Loading + parsing the tracker DSL into a `Score`: `from_env` (resolve the file / fall back to the
//! built-in) and `from_str` (the line-by-line parser), plus the small token parsers. The reverse
//! direction (a `Score` back to text) is in `dump`; the structural lint in `validate`.

use super::types::{Chord, Ramp, Section};
use super::validate::{strict_scores, validate};
use super::{DEFAULT_SCORE, Score};

impl Score {
    /// `MARTIN_SCORE=<file>` loads a tracker-DSL score; on any error we log + fall back to the
    /// built-in, so a bad score file never stops the show.
    pub fn from_env() -> Score {
        // MARTIN_SCORE override, else the editable default file (edit it → no recompile), else the
        // embedded built-in (a bundled binary with no assets/ folder).
        let path = std::env::var("MARTIN_SCORE")
            .ok()
            .filter(|p| !p.is_empty())
            .or_else(|| {
                std::path::Path::new(DEFAULT_SCORE)
                    .exists()
                    .then(|| DEFAULT_SCORE.to_string())
            });
        let Some(path) = path else {
            return Score::builtin();
        };
        match std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|t| Score::from_str(&t))
        {
            Ok(s) => {
                eprintln!(
                    "score: {path} ({} sections, {:.0}s)",
                    s.sections.len(),
                    s.demo_len()
                );
                // structural lint: surface the DSL's silent traps (phase/bar mismatch, a pattern on a
                // phase the section lacks, an ignored melodic p1+). Warnings don't fail the load — the
                // show still plays — UNLESS `MARTIN_SCORE_STRICT` is set (authoring / CI), then they're
                // fatal so a broken score can't slip through.
                let warnings = validate(&s.sections);
                for w in &warnings {
                    eprintln!("score: warning: {w}");
                }
                if !warnings.is_empty() && strict_scores() {
                    eprintln!(
                        "score: {} warning(s) with MARTIN_SCORE_STRICT — aborting",
                        warnings.len()
                    );
                    std::process::exit(1);
                }
                s
            }
            Err(e) => {
                eprintln!("score: {path}: {e} — using embedded built-in");
                Score::builtin()
            }
        }
    }

    /// Parse a tracker-DSL score (see `to_dsl` for the shape / `USAGE.md` for the grammar).
    pub fn from_str(text: &str) -> Result<Score, String> {
        let mut bpm = 140.0_f32;
        let mut chords: Vec<Chord> = Vec::new();
        let mut sections: Vec<Section> = Vec::new();
        let mut params: std::collections::HashMap<String, f32> = std::collections::HashMap::new();
        let find = |sections: &[Section], name: &str| sections.iter().position(|s| s.name == name);

        for (n, raw) in text.lines().enumerate() {
            // Strip a `# comment`, but only when `#` starts the line or follows whitespace — so a
            // sharp note like `F#6` (mid-token `#`) is NOT mistaken for a comment.
            let bytes = raw.as_bytes();
            let cut = raw
                .char_indices()
                .find(|&(i, c)| c == '#' && (i == 0 || bytes[i - 1].is_ascii_whitespace()))
                .map(|(i, _)| i)
                .unwrap_or(raw.len());
            let line = raw[..cut].trim();
            if line.is_empty() {
                continue;
            }
            let ln = n + 1;
            // never `.unwrap()` on parsed input: a non-empty trimmed line has a token, but stay
            // defensive so a weird line skips rather than panics.
            let Some(first) = line.split_whitespace().next() else {
                continue;
            };

            // pattern line: `<section>.<inst> p<N>|fill: <16 steps>`
            if first.contains('.') {
                // per-section knob override: `<section>.set key=value …` (no colon — mirrors global
                // `set`; the synth reads it via `param_at` inside that section only).
                if let Some((sec, "set")) = first.split_once('.') {
                    let si = find(&sections, sec)
                        .ok_or_else(|| format!("line {ln}: unknown section `{sec}`"))?;
                    for tok in line.split_whitespace().skip(1) {
                        let (k, v) = tok.split_once('=').ok_or_else(|| {
                            format!("line {ln}: `{sec}.set` needs key=value, got `{tok}`")
                        })?;
                        let val = v
                            .parse()
                            .map_err(|_| format!("line {ln}: bad set value `{tok}`"))?;
                        sections[si].params.insert(k.to_string(), val);
                    }
                    continue;
                }
                let (head, pat) = line
                    .split_once(':')
                    .ok_or_else(|| format!("line {ln}: pattern needs a ':'"))?;
                let mut h = head.split_whitespace();
                // a line like `: x...` has an empty target before the ':' — a real malformed-input
                // case that used to panic; report it instead.
                let target = h
                    .next()
                    .ok_or_else(|| format!("line {ln}: empty `section.inst` before ':'"))?;
                let phase_tok = h.next().unwrap_or("p0");
                let (sec, inst) = target
                    .split_once('.')
                    .ok_or_else(|| format!("line {ln}: expected `section.inst`, got `{target}`"))?;
                let si = find(&sections, sec)
                    .ok_or_else(|| format!("line {ln}: unknown section `{sec}`"))?;
                let phase: Option<usize> = if phase_tok.eq_ignore_ascii_case("fill") {
                    None
                } else {
                    Some(
                        phase_tok
                            .trim_start_matches('p')
                            .parse()
                            .map_err(|_| format!("line {ln}: bad phase `{phase_tok}`"))?,
                    )
                };
                if inst == "chords" {
                    // per-section chord override: `<section>.chords: G Am Bb D` (cycles in-section).
                    let mut cs = Vec::new();
                    for tok in pat.split_whitespace() {
                        cs.push(
                            parse_chord(tok)
                                .ok_or_else(|| format!("line {ln}: bad chord `{tok}`"))?,
                        );
                    }
                    sections[si].chords = cs;
                } else if inst == "fx" {
                    // per-section FX/layer selection: `<section>.fx: wall jet impact` (overrides the
                    // built-in name-based defaults for that section). An empty list = no FX at all.
                    sections[si].fx = Some(pat.split_whitespace().map(|s| s.to_string()).collect());
                } else if inst == "lead" || inst == "arp" || inst == "bass" {
                    // pitched note lane: a phrase of 1+ bars (16 note tokens each, `A4`/`C#5`/`.`).
                    let grid = parse_notes(pat).ok_or_else(|| {
                        format!("line {ln}: {inst} needs 16 notes/rests (or a multiple of 16)")
                    })?;
                    let lane = match inst {
                        "arp" => &mut sections[si].arp,
                        "bass" => &mut sections[si].bass,
                        _ => &mut sections[si].lead,
                    };
                    match phase {
                        None => lane.fill = grid,
                        Some(p) => {
                            if lane.phases.len() <= p {
                                lane.phases.resize(p + 1, Vec::new());
                            }
                            lane.phases[p] = grid;
                        }
                    }
                } else {
                    let grid = parse_pattern(pat)
                        .ok_or_else(|| format!("line {ln}: pattern must be 16 of x/."))?;
                    let lane = sections[si]
                        .lane_mut(inst)
                        .ok_or_else(|| format!("line {ln}: unknown instrument `{inst}`"))?;
                    match phase {
                        None => lane.fill = grid,
                        Some(p) => {
                            if lane.phases.len() <= p {
                                lane.phases.resize(p + 1, [false; 16]);
                            }
                            lane.phases[p] = grid;
                        }
                    }
                }
                continue;
            }

            let mut it = line.split_whitespace();
            let Some(kw) = it.next() else {
                continue; // unreachable (line is non-empty), but never unwrap on parsed input
            };
            match kw {
                "bpm" => {
                    bpm = it
                        .next()
                        .and_then(pf)
                        .ok_or_else(|| format!("line {ln}: bpm needs a number"))?;
                }
                "section" => {
                    let name = it
                        .next()
                        .ok_or_else(|| format!("line {ln}: section needs a name"))?
                        .to_string();
                    let bars: u32 = it
                        .next()
                        .and_then(|x| x.parse().ok())
                        .ok_or_else(|| format!("line {ln}: section needs a bar count"))?;
                    let mut phases = vec![bars];
                    let mut fill = false;
                    for tok in it {
                        if tok.eq_ignore_ascii_case("fill") {
                            fill = true;
                        } else {
                            let ph: Vec<u32> =
                                tok.split(',').filter_map(|x| x.parse().ok()).collect();
                            if !ph.is_empty() {
                                phases = ph;
                            }
                        }
                    }
                    sections.push(Section::empty(name, bars, phases, fill));
                }
                "chords" => {
                    for tok in it {
                        chords.push(
                            parse_chord(tok)
                                .ok_or_else(|| format!("line {ln}: bad chord `{tok}`"))?,
                        );
                    }
                }
                "gain" | "sub" | "mids" => {
                    let toks: Vec<&str> = it.collect();
                    for pair in toks.chunks(2) {
                        let [name, val] = pair else { break };
                        let si = find(&sections, name)
                            .ok_or_else(|| format!("line {ln}: unknown section `{name}`"))?;
                        let r = parse_ramp(val)
                            .ok_or_else(|| format!("line {ln}: bad value `{val}`"))?;
                        match kw {
                            "gain" => sections[si].gain = r,
                            "sub" => sections[si].sub = r,
                            _ => sections[si].mids = r,
                        }
                    }
                }
                // `set lead=0.82 reverb=0.35 ...` — free-form mix/fx knobs the synth reads (with its
                // own defaults). Lets the SOUND be tuned by editing the score, not recompiling.
                "set" => {
                    for tok in it {
                        let (k, v) = tok.split_once('=').ok_or_else(|| {
                            format!("line {ln}: `set` needs key=value, got `{tok}`")
                        })?;
                        let val = v
                            .parse()
                            .map_err(|_| format!("line {ln}: bad set value `{tok}`"))?;
                        params.insert(k.to_string(), val);
                    }
                }
                other => return Err(format!("line {ln}: unknown keyword `{other}`")),
            }
        }
        if sections.is_empty() {
            return Err("no sections defined".into());
        }
        let mut score = Score::new(bpm, chords, sections);
        score.params = params;
        Ok(score)
    }

    /// The default score: the **embedded** `assets/score.txt`, so the notes / patterns / chords
    /// live in the editable text file, not in code. `from_env` prefers the on-disk copy when it's
    /// present (edit it → no recompile); this embedded copy is the fallback a bundled binary ships.
    pub fn builtin() -> Score {
        Score::from_str(include_str!("../../assets/score.txt"))
            .expect("embedded assets/score.txt must parse")
    }
}

// ---- token parsers -------------------------------------------------------------------------

/// Leading-dot-tolerant float parse (`.85` → 0.85).
fn pf(s: &str) -> Option<f32> {
    let s = s.trim();
    s.parse().ok().or_else(|| format!("0{s}").parse().ok())
}

fn parse_ramp(s: &str) -> Option<Ramp> {
    match s.split_once('>') {
        Some((a, b)) => Some(Ramp::new(pf(a)?, pf(b)?)),
        None => Some(Ramp::c(pf(s)?)),
    }
}

/// Parse a note name → frequency (Hz): letter `A`–`G`, optional `#`/`b`, octave (`A4` = 440 Hz).
pub(super) fn note_freq(name: &str) -> Option<f32> {
    let mut chars = name.chars();
    let base: i32 = match chars.next()?.to_ascii_uppercase() {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let mut semi = base;
    let mut rest = chars.as_str();
    match rest.chars().next() {
        Some('#') => {
            semi += 1;
            rest = &rest[1..];
        }
        Some('b') => {
            semi -= 1;
            rest = &rest[1..];
        }
        _ => {}
    }
    let octave: i32 = rest.parse().ok()?;
    let midi = (octave + 1) * 12 + semi;
    Some(440.0 * 2f32.powf((midi as f32 - 69.0) / 12.0))
}

/// Parse a chord token: a note (letter + optional `#`/`b`) + optional trailing `m` for minor
/// (`Am`, `F`, `C#`, `Ebm`). The root is taken at octave 3.
fn parse_chord(s: &str) -> Option<Chord> {
    let s = s.trim();
    let (note, minor) = match s.strip_suffix('m') {
        Some(p) if !p.is_empty() => (p, true),
        _ => (s, false),
    };
    Some(Chord {
        root: note_freq(&format!("{note}3"))?,
        minor,
    })
}

/// Parse whitespace-separated note tokens (`A4`/`C#5`/… or `.`/`-`/`_` = rest) into a melodic
/// phrase: one or more bars of 16 slots each (so a 32/48/… token line is a 2/3/…-bar phrase). The
/// token count must be a positive multiple of 16.
fn parse_notes(s: &str) -> Option<Vec<[Option<f32>; 16]>> {
    let toks: Vec<&str> = s.split_whitespace().collect();
    if toks.is_empty() || !toks.len().is_multiple_of(16) {
        return None;
    }
    let mut bars = Vec::with_capacity(toks.len() / 16);
    for chunk in toks.chunks(16) {
        let mut bar = [None; 16];
        for (i, t) in chunk.iter().enumerate() {
            bar[i] = match *t {
                "." | "-" | "_" => None,
                n => Some(note_freq(n)?),
            };
        }
        bars.push(bar);
    }
    Some(bars)
}

/// Parse a 16-step grid: `x`/`X` = hit, `.`/`-`/`_` = rest; spaces / `|` group separators ignored.
fn parse_pattern(s: &str) -> Option<[bool; 16]> {
    let mut out = [false; 16];
    let mut i = 0;
    for c in s.chars() {
        match c {
            ' ' | '\t' | '|' => {}
            'x' | 'X' => {
                *out.get_mut(i)? = true;
                i += 1;
            }
            '.' | '-' | '_' => {
                *out.get_mut(i)? = false;
                i += 1;
            }
            _ => return None,
        }
    }
    (i == 16).then_some(out)
}
