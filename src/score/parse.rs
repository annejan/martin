//! Loading + parsing the tracker DSL into a `Score`: `from_env` (resolve the file / fall back to the
//! built-in) and `from_str` (the line-by-line parser), plus the small token parsers. The reverse
//! direction (a `Score` back to text) is in `dump`; the structural lint in `validate`.

use super::types::{Chord, Ramp, Section, TempoPoint};
use super::validate::{strict_scores, validate};
use super::{DEFAULT_SCORE, Score};

/// Upper bound on a `pN` phase index (it directly sizes/indexes a lane's phase Vec). A section never
/// has more than a handful of phases; this caps a malformed `p999999999` before it triggers a
/// multi-GB resize or a `usize` overflow. See `MAX_BARS` for the matching whole-track guard.
const MAX_PHASE: usize = 64;
/// Upper bound on a section's bar count (a runaway `section x 4000000000` would make the synth-prep
/// walks iterate `bars·16` slots into an unbounded Vec → hang/OOM). 10k bars ≈ hours of music.
const MAX_BARS: u32 = 10_000;
/// Upper bound on a section's `grid:N` (slots per bar). Generous — a whole odd-meter riff cycle as one
/// bar (e.g. Golden Brown's |3/4 3/4 3/4 4/4| triplet grid) is well under this; caps the per-bar Vec.
const MAX_GRID: usize = 256;

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
                let mut warnings = validate(&s.sections);
                warnings.extend(s.tempo_warnings());
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
        let mut swing = 0.0_f32; // global swing default (the `swing N` line)
        let mut tempo: Vec<TempoPoint> = Vec::new();
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
                    let p: usize = phase_tok
                        .trim_start_matches('p')
                        .parse()
                        .map_err(|_| format!("line {ln}: bad phase `{phase_tok}`"))?;
                    if p > MAX_PHASE {
                        return Err(format!(
                            "line {ln}: phase p{p} out of range (max p{MAX_PHASE})"
                        ));
                    }
                    Some(p)
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
                    // pitched note lane: a phrase of 1+ bars, 16 tokens each — `A4`/`C#5` note, `.`
                    // rest, `-`/`_` TIE (hold the previous note one more slot: `C4 - - .` = a dotted-
                    // eighth-ish held C). A note with no ties is unchanged (the fixed slot length).
                    let g = sections[si].grid;
                    let phrase = parse_notes(pat, g).ok_or_else(|| {
                        format!(
                            "line {ln}: {inst} needs {g} notes/rests per bar (a multiple of {g})"
                        )
                    })?;
                    let lane = match inst {
                        "arp" => &mut sections[si].arp,
                        "bass" => &mut sections[si].bass,
                        _ => &mut sections[si].lead,
                    };
                    match phase {
                        None => lane.fill = phrase,
                        Some(p) => {
                            if lane.phases.len() <= p {
                                lane.phases.resize(p + 1, Vec::new());
                            }
                            lane.phases[p] = phrase;
                        }
                    }
                } else {
                    let g = sections[si].grid;
                    let pattern = parse_pattern(pat, g)
                        .ok_or_else(|| format!("line {ln}: pattern must be {g} of x/."))?;
                    let lane = sections[si]
                        .lane_mut(inst)
                        .ok_or_else(|| format!("line {ln}: unknown instrument `{inst}`"))?;
                    match phase {
                        None => lane.fill = pattern,
                        Some(p) => {
                            if lane.phases.len() <= p {
                                lane.phases.resize(p + 1, Vec::new());
                            }
                            lane.phases[p] = pattern;
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
                    if bpm <= 0.0 || bpm.is_nan() {
                        return Err(format!("line {ln}: bpm must be > 0 (got {bpm})"));
                    }
                }
                // `swing N` — global swing (0 = straight, 1 = exact triplet feel). Per-section
                // `swing:N` on a section line overrides it. Absent = straight (byte-identical).
                "swing" => {
                    swing = it
                        .next()
                        .and_then(pf)
                        .ok_or_else(|| format!("line {ln}: swing needs a number"))?;
                    if !(0.0..=1.0).contains(&swing) {
                        return Err(format!(
                            "line {ln}: swing must be 0..=1 (0=straight, 1=triplet), got {swing}"
                        ));
                    }
                }
                // `tempo @bar:8=96 @bar:16=132 ...` — tempo CHANGES at bar boundaries (rubato). `bpm`
                // stays the bar-0 default; each pair steps the tempo from that bar on. Sorted/deduped
                // below; absent ⇒ constant tempo (byte-identical to a plain `bpm N` score).
                "tempo" => {
                    for tok in it {
                        let (bar_tok, bpm_tok) = tok.split_once('=').ok_or_else(|| {
                            format!("line {ln}: tempo needs @bar:N=BPM, got `{tok}`")
                        })?;
                        let bar: u32 = bar_tok
                            .trim_start_matches('@')
                            .strip_prefix("bar")
                            .map(|s| s.trim_start_matches(':'))
                            .and_then(|s| s.parse().ok())
                            .ok_or_else(|| {
                                format!("line {ln}: tempo position must be @bar:N, got `{bar_tok}`")
                            })?;
                        let b = pf(bpm_tok).ok_or_else(|| {
                            format!("line {ln}: tempo bpm not a number `{bpm_tok}`")
                        })?;
                        if b <= 0.0 || b.is_nan() || b > 1000.0 {
                            return Err(format!(
                                "line {ln}: tempo bpm must be in (0, 1000] (got {b})"
                            ));
                        }
                        tempo.push(TempoPoint { bar, bpm: b });
                    }
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
                    if bars == 0 || bars > MAX_BARS {
                        return Err(format!(
                            "line {ln}: section `{name}` has {bars} bars (max {MAX_BARS}) — the \
                             whole-track walks would hang/OOM"
                        ));
                    }
                    let mut phases = vec![bars];
                    let mut fill = false;
                    let mut grid = 16usize;
                    let mut sec_swing = f32::NAN; // NAN = inherit the global `swing` (resolved below)
                    for tok in it {
                        if tok.eq_ignore_ascii_case("fill") {
                            fill = true;
                        } else if let Some(g) = tok.strip_prefix("grid:") {
                            // odd meter / tuplets: slots per bar (a slot stays a 16th, so the bar is
                            // grid/4 beats). `grid:12` = a 3/4 bar; default 16 = 4/4.
                            grid = g
                                .parse()
                                .ok()
                                .filter(|&g| (1..=MAX_GRID).contains(&g))
                                .ok_or_else(|| {
                                    format!("line {ln}: grid must be 1..={MAX_GRID}, got `{g}`")
                                })?;
                        } else if let Some(v) = tok.strip_prefix("swing:") {
                            // per-section swing override (0 = straight, 1 = triplet).
                            sec_swing =
                                pf(v).filter(|x| (0.0..=1.0).contains(x)).ok_or_else(|| {
                                    format!("line {ln}: swing: must be 0..=1, got `{v}`")
                                })?;
                        } else {
                            let ph: Vec<u32> =
                                tok.split(',').filter_map(|x| x.parse().ok()).collect();
                            if !ph.is_empty() {
                                phases = ph;
                            }
                        }
                    }
                    let mut sec = Section::empty(name, bars, phases, fill);
                    sec.grid = grid;
                    sec.swing = sec_swing;
                    sections.push(sec);
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
        // sorted ascending by bar; first pair wins on a duplicate bar (the MIDI tool emits one per bar).
        tempo.sort_by_key(|p| p.bar);
        tempo.dedup_by_key(|p| p.bar);
        // a section with no `swing:` (NaN) inherits the global `swing` — resolve before Score::new,
        // which reads each section's swing to build the per-bar table.
        for s in &mut sections {
            if s.swing.is_nan() {
                s.swing = swing;
            }
        }
        let mut score = Score::new(bpm, tempo, chords, sections);
        score.params = params;
        score.swing = swing;
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

/// Leading-dot-tolerant float parse (`.85` → 0.85, `-.85` → -0.85).
fn pf(s: &str) -> Option<f32> {
    let s = s.trim();
    s.parse().ok().or_else(|| {
        if let Some(rest) = s.strip_prefix("-.") {
            format!("-0.{rest}").parse().ok()
        } else if s.starts_with('.') {
            format!("0{s}").parse().ok()
        } else {
            None
        }
    })
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
    // any real note is octave 0–9; reject absurd values BEFORE the arithmetic so a garbage token
    // (`A2147483647`) can't overflow `(octave+1)*12` (debug/test panic; release wraps to a non-finite
    // frequency fed to the synth). `None` → the lane parser turns it into a clean "needs 16 notes" error.
    if !(-1..=10).contains(&octave) {
        return None;
    }
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
/// Parse pitched note lanes (`lead`/`arp`/`bass`): whitespace tokens, `grid` per bar. `A4`/`C#5` = a
/// note, `.` = rest, `-`/`_` = a **tie** (held as `NaN`, merged into the preceding note's duration by
/// `Score::note_line`). Returns one `Vec<Option<f32>>` (length `grid`) per bar.
fn parse_notes(s: &str, grid: usize) -> Option<Vec<Vec<Option<f32>>>> {
    let toks: Vec<&str> = s.split_whitespace().collect();
    if toks.is_empty() || grid == 0 || !toks.len().is_multiple_of(grid) {
        return None;
    }
    let mut bars = Vec::with_capacity(toks.len() / grid);
    for chunk in toks.chunks(grid) {
        let mut bar = Vec::with_capacity(grid);
        for t in chunk {
            bar.push(match *t {
                "." => None,                 // rest
                "-" | "_" => Some(f32::NAN), // TIE: hold the previous note through this slot
                n => Some(note_freq(n)?),
            });
        }
        bars.push(bar);
    }
    Some(bars)
}

/// Parse a `grid`-step pattern: `x`/`X` = hit, `.`/`-`/`_` = rest; spaces / `|` group separators
/// ignored. Returns a `Vec<bool>` of exactly `grid` steps.
fn parse_pattern(s: &str, grid: usize) -> Option<Vec<bool>> {
    let mut out = vec![false; grid];
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
    (i == grid).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::note_freq;

    #[test]
    fn note_freq_rejects_absurd_octave_without_overflowing() {
        assert!(note_freq("A4").is_some()); // a real note
        assert!(note_freq("C0").is_some());
        // absurd octaves return None (the lane parser turns that into a clean error) instead of
        // overflowing `(octave+1)*12` → a debug/test panic / release garbage frequency.
        assert_eq!(note_freq("A2147483647"), None);
        assert_eq!(note_freq("A-2147483648"), None);
        assert_eq!(note_freq("A99"), None);
    }
}
