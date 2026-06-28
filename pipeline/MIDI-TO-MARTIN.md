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

The score DSL is **fixed 4/4 with a 16-slot (16th-note) grid**. The `16` is the array *type* of every
note/drum bar (`[Option<f32>; 16]`, `[bool; 16]`) — wired through
`src/score/{parse,types,dump,validate,mod}.rs` + `src/audio/render.rs`. Tempo automation is **done**
(see below); odd meter/triplets remain the boundary.

### Golden Brown (The Stranglers) — odd meter + triplets
- It alternates **3/4 ×3 then 4/4** (a 13/8 feel) and the signature harpsichord is in **triplets**.
- On a 4/4 16th grid the triplets quantise to 16ths and the bars don't fit the meter → the notes are
  right but the **rhythm/swing shifts**, so it's not recognisable.
- **To fix:** a per-score subdivision (e.g. `grid 12`/`24` for triplets) AND a variable
  beats-per-bar / time-signature. **Scope:** a real refactor — the fixed `[…; 16]` arrays become a
  flexible length (Vec or const-generic) across the whole score subsystem, plus the timing math and the
  section/anchor maths. Doable but a deliberate ~1-2 day project with careful regression of every
  existing score (camping/intro/beach/… all parse via the `[;16]` arrays); gate it behind a per-score
  opt-in (default 16) so existing content stays byte-identical. **Risk: medium-high.**

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
