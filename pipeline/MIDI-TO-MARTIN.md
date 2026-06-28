<!--
SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
SPDX-License-Identifier: MIT
-->

# MIDI → martin: turning songs into scores

`pipeline/midi_to_martin.py` converts a song MIDI into a martin `.txt` score (the tracker DSL) in one
command. It bakes in the lessons from the **jantje** project (the separate MIDI→C64-SID arranger) so the
output sounds like the song instead of mush — see `memory/jantje-music-lessons.md` for *why* each rule
exists, and `midi_inspect.py` (in jantje) for reading a MIDI's track names.

## Quick use

```sh
# auto-detect everything, a faithful playthrough at the song's own tempo
pipeline/midi_to_martin.py song.mid out.txt --faithful --style synthpop
MARTIN_SCORE=out.txt ./target/release/martin --synth-wav out.wav

# print the detected role map (a dry run) — no out file
pipeline/midi_to_martin.py song.mid
```

| flag | what |
|---|---|
| (default) | a clean **dance remix**: a looped 16-bar block staged into sections (`--arrange song\|short\|dance`) |
| `--faithful` | play the song **through once** at its own tempo: lead + bass + harmony + the real drums, natural mix |
| `--style` | voice/mix palette: `clean` (default) · `synthpop` · `dance` · `rock` · `orchestral` |
| `--no-drums` | omit the kit (solo-piano / ambient — no house floor over it) |
| `--no-meter` | ignore odd time signatures — force-fit the song to 4/4 (default: read FF 58, emit `grid:N`) |
| `--no-tempo-map` | ignore the MIDI's tempo changes — render at one steady tempo |
| `--vocal/--bass/--fill N` | force a 1-based channel when the auto-detect picks wrong |
| `--bpm N` | override the tempo (also rescues a misread tempo meta) |

## How the auto-detect works (the jantje rules)

- **Lead = the channel LABELLED the melody** (`Melody`/`Vocal`/`CANTO`, or a lead GM patch) — *not* the
  highest-pitched one. With nothing labelled (a GM instrumental) it falls back to register + a note-
  density sweet-spot (so the actual tune wins over a sparse riff or a busy comp).
- **Bass** = the labelled/low channel; **drums** = GM channel 10; **fill** = the busiest remaining
  *melodic* channel (percussion patches GM 112-119 and extreme-high ostinati are excluded — they clang).
- **Fill** drops the riff ONLY into the vocal's whole-bar rest holes (it answers her phrases), never
  crammed over a busy vocal.
- **Single-channel** (a solo piano) → the lead channel is **pitch-split**: low notes (LH) become the
  bass, high notes (RH) stay the lead. **No drum channel** → a four-on-the-floor fallback (or `--no-drums`).
- per-bar chords are read from the bass so the pad/stab follow the harmony.

Tested clean on: Björk *Human Behaviour*, Haddaway *What Is Love*, R.E.M. *Shiny Happy People*, the
*Godfather* theme, Rick Astley, *Never Ending Story*, *You Spin Me Round*.

## Known limits — `martin`'s score model (the "pick up later" list)

The score DSL grid is a 16th-note grid. Both big limits are now **done**: tempo automation (rubato)
and variable grid (odd meter) — see below. The only remaining boundary is genuine *triplets within a
beat* that aren't a compound-meter bar (rare); those still quantise to 16ths.

### Golden Brown (The Stranglers) — odd meter — ✅ DONE (variable grid)
- It alternates **3/4 ×3 then 4/4** (the 13-beat cycle).
- **Fixed:** the score DSL now has a per-section `grid:N` (slots per bar, default 16) — `grid:12` is a
  3/4 bar, `grid:16` a 4/4 bar — and `--faithful` reads the MIDI's **time signatures** (FF 58) and emits
  one section per bar at the right grid (so a non-4/4 source auto-routes to the variable-grid renderer).
  The per-bar slot/note arrays became `Vec` and the timeline carries a cumulative `bar_slot0` table;
  an all-16-grid score is byte-identical to before. The tempo map composes on top, so a rubato
  compound-meter piece (Clair de Lune in 6/8↔9/8) keeps both its meter AND its breathing.
  `--no-meter` force-fits to 4/4. See `USAGE.md` § The score file (`grid:N`).
- **Still 4/4-grid-quantised** (not a meter issue): genuine *triplets within a beat* that aren't a
  compound-meter bar. Most "odd" pieces are meter, which this covers.

### Clair de Lune (Debussy) — rubato — ✅ DONE (tempo automation)
- An impressionist solo-piano piece that lives on **rubato** (freely pushing/pulling the tempo).
- **Fixed:** the score DSL now has a `tempo @bar:N=BPM …` line (piecewise-constant per bar) and
  `--faithful` emits it automatically from the MIDI's set-tempo events. The slot↔seconds map is
  piecewise-per-bar, so the timing breathes — a slow bar is literally longer — and both the synth and
  the `@@anchor`s follow it. A score with no `tempo` line is byte-identical to before. See `USAGE.md`
  § The score file. (A "nicer Clair de Lune" still wants softer, more piano-like lead/pad voices than
  the `orchestral` palette — that's a voice job, not a timing one.)
- **Still deferred** (constant-tempo dance scores are unaffected today): the procedural dance layers
  (wall/shimmer/stab/build walks in `render.rs`) use the nominal tempo, and tempo is bar-stepped (no
  true intra-bar linear ramp). A rubato piece with those layers would desync them; classical
  (notes + pad, no drums) is fully covered.

**Verdict:** **4/4 at a steady tempo** (most pop/dance/rock) renders well, and **4/4 rubato** (classical
transcriptions) now breathes via the tempo map. Odd meters / triplets (Golden Brown) are the remaining
boundary — that's the deliberate grid refactor above.
