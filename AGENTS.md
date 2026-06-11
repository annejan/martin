# Op de Camping

## Goal
Transform the "Op de Camping" (Ome Henk, 1995) demoscene track from a basic placeholder into a full-spectrum audio experience. All melodic lines (lead, arp, bass) must remain unchanged — only drums, dynamics, spatiality, and camera are free to modify.

## File Structure
- `assets/score.txt`: Tracker DSL — BPM, sections, chords, pattern tables, per-drum-lane hit patterns, gain/sub/mids curves. Editable at runtime (no recompile).
- `src/audio.rs`: FunDSP voice synthesis — kick, snare, hat, lead, arp, bass, stab, pad, sub, reverb, sidechain, master limiter. Requires recompile.
- `assets/op-de-camping.show`: Unified show format — camera track, text sequence with @@anchors to score sections.
- `src/score.rs`: Parser for score.txt — resolves section timing, drum hits, note lanes, phase lookup.
- `src/music.rs`: Bevy plugin binding between score and audio playback.

## Build & Render
- `cargo run --release` runs the full Bevy app (with visuals).
- `MARTIN_SYNTH_WAV=<path.wav> cargo run --release` renders WAV only (headless, no visuals).
- `ffmpeg -i <input.wav> -b:a 320k <output.mp3>` converts to MP3.
- Typically ~228s at 44.1 kHz, ~3.5 min track.

## Score DSL (score.txt)
- `bpm <bpm>` 
- `chords: <chord list>` — global chord progression, cycles per bar.
- `section <name> <bars> <phase-bars,csv> [fill]` — defines sections. Phase bars are comma-separated (e.g. `8,8,8,8 fill`). `fill` means the last bar is a fill.
- `section.chords:` — per-section chord override.
- Per-section drum patterns: `<section>.kick|snare|hat|stab p0|p1|p2|p3|fill: <16 steps (x=hit .=rest)>`
- Per-section melodic lanes: `<section>.lead|arp|bass p0: <16 notes per bar, multi-bar phrase>`
  - Melody loops continuously ignoring drum phase boundaries (uses only p0).
  - Drum phases look up by index: `phase_at(bar_into_section)` returns which phase (0,1,2,3,255=fill).
  - Undefined phase = silence `[false; 16]`.
- Dynamics: `gain|sub|mids <section> <value_or_ramp>` — e.g. `gain build 0.25>1.1` ramps across section.

## Audio Architecture (audio.rs)
- **Kick**: `render_hardkick` — hardstyle/gabber approach: pitched sine body sweep + tonal tail on chord root + click transient. Rendered to own buffer (never ducked by sidechain). Level: `0.92 * (0.9 + 0.1 * vel(...))`.
- **Snare**: Enhanced with clap body layer (280 Hz low-passed noise, slower decay). ±0.2 pan spread.
- **Hat**: Enhanced with body layer (3.5 kHz noise layer). ±0.65 pan spread.
- **Lead**: Main melody. Was 0.34 → bumped to 0.50 (currently 0.55). Octave sheen at 0.15 (was 0.12). Climax extra sheen at 0.18 (was 0.14). Haas widening (12 ms R-channel offset, 600 Hz–6 kHz band-limited).
- **Arp**: Ping-pong delay (8th-note, bounces L-R, separate buffer). ±0.7 pan. Arp volume 0.20 (was 0.15).
- **Bass**: Kick reinforcement + articulated bassline on top of continuous sub drone.
- **Pad**: Auto-pan (slow LFO, 0.4× BPM sine).
- **Stab**: Chord spread ±0.75.
- **Reverb**: Wet 0.35, comb feedback 0.88.
- **Sidechain**: Depth 0.7, recovery 0.08 — triggered by kick buffer.
- **Limiter**: Threshold 0.93 with soft-clip. Gain > 1.0 engages compression.
- **Mid-side**: Widening factor 1.55.

## Section Structure
- intro: 8 bars (single phase)
- build: 17 bars (8,8 fill) — verse (G-minor)
- drop: 33 bars (8,24 fill) — chorus (G-major)
- breakdown: 17 bars (8,8 fill) — verse (G-minor)
- climax: 33 bars (8,8,8,8 fill) — chorus (G-major), p3 = fake-out breather
- outro: 25 bars (8,8,8 fill) — chorus (G-major), escalating finale

## Key Decisions
- Melody is sacred — comes from MIDI transcription of the actual song, never change lead/arp/bass note data.
- Fills are one bar of max intensity — intentionally over the top.
- DnB two-step foundation: kick on 1 + "and of 3" (step 11), snare on 2 & 4 (steps 5 & 13).
- Section gain curves should have wide contrast (intro quiet → drop/climax/outro pushing limiter).
- Sub bass should drone through breakdown (sub ~0.55, gain ~0.07, mids ~0.08).

## Visuals & Camera
- Show file controls camera position/movement and text overlay sequence.
- Camera was changed from static `pos=0,0,0` to 3D movement: arcs, flybys, push-ins, blast-back on finale.
- Camera responds to kick beat-pump (slight scale/position pulse).

## Testing
- `cargo test` — 54 tests pass.
- `MARTIN_SCORE_DUMP=<path>` writes normalized score dump for debugging.

## Common Pitfalls
- Changing phase bars (`8,8` → `4,4,8`) without adding corresponding p0/p1/p2 drum patterns = missing patterns = silence.
- Changing hat patterns from swung to straight kills the DnB groove.
- Breaking the fill bar count (phases sum + 1 fill must equal section bars).
- Missing `p1` patterns in sections that use them (build uses p0+p1, drop uses p0+p1, breakdown uses p0+p1, climax uses p0+p1+p2+p3, outro uses p0+p1+p2).
