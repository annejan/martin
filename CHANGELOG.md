<!--
SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
SPDX-License-Identifier: MIT
-->

# Changelog

All notable changes to martin. Format follows [Keep a Changelog](https://keepachangelog.com/);
the project has no tagged releases yet, so everything lives under **Unreleased**.

## [Unreleased]

### Engine
- Sequence engine: a timeline of *parts* (`text:` / `splat:` / `mesh:` / `glb:` / `image:` / `wall:`)
  that assemble out of a ball cloud and morph into the next, per-Gaussian on the GPU, with a directed
  camera track. Composed via `MARTIN_*` env vars or a single unified `.show` file (`MARTIN_SHOW`).
- mesh→splat sampling: density-adaptive disk size + R2 low-discrepancy distribution, per-splat
  translucency (`MARTIN_MESH_OPACITY`), and a glTF (`.glb`) loader.
- Self-contained single-binary bundle (`--features bundle`): show assets are lz4-embedded and
  self-extract at startup.
- `KHR_gaussian_splatting` glTF loading (`MARTIN_GLB=<file.glb>`): render a standard-container splat
  scene (e.g. a TRELLIS single-image→3DGS export) through the normal bloom pipeline — distinct from
  the `glb:`/`model:` *mesh* paths. `MARTIN_GLB_SCALE` / `MARTIN_GLB_DIST` size + frame it.
- Per-part backgrounds: the `bg:<name>` seq token switches the fullscreen background shader from
  that part on (sticky; `bg:off` = pure black) — the background becomes a second energy curve.
- `out:explode` (`out:burst`) departure: a part is flung ballistically outward from its centre and
  fades — a real burst, punchier than `out:disperse`'s wash. Pairs with a `glb:` dissolve for a
  mesh → blob → explode exit.
- **Domain-driven authoring** (see `DOMAIN.md`): the `.show` DSL now speaks the domain vocabulary —
  `[reel]` (was `[seq]`), `[stage]` (was `[compose]`), `~entrance`, `exit:` (was `out:`), `flock:`
  (was `cluster:`), `backdrop:` (was `bg:`), `budget=` (was `morph_count=`). All old spellings keep
  parsing as aliases. Plus a production **kind**: `kind = intro|demo`. An `intro` is self-contained +
  asset-budgeted (bundles into the single binary); `MARTIN_VALIDATE` reports its asset budget and
  warns on heavy / missing / capture-only assets. A `demo` is full-fat (local captures allowed).
- **`[scenes]` authoring** — write a show as the Showbook **arc** of named **Scenes** instead of a flat
  reel: each `scene` line opens a beat and sets its look (`@@anchor` / `backdrop:` / `^deform`), which
  the Shots under it inherit (a Shot's own modifier wins). Flattens to the exact `[reel]` the engine
  already runs — pure sugar, content-agnostic. Example: `assets/examples/arc.show`. (`[arc]` aliases it.)
- Raster modes (`raster:<mode>` per-part token + `MARTIN_RASTER` global default): expose the fork's
  RasterizeMode debug-shading views — `color`/`depth`/`normal`/`position`/`classification`/`flow`/
  `velocity`. `position` colours each gaussian by XYZ (a rainbow gradient) — e.g.
  `text:deFEEST ~outline raster:position` reveals the letters in a position-colour rainbow.
- SH build profiles: `sh0` (default, flat colour) and `sh3` (degree-3 view-dependent glint, for real
  captures) — `cargo b-sh3` builds into a separate target dir so both binaries coexist.
- `MARTIN_PREVIEW_FPS=<n>`: render the timeline at n fps instead of 60 — far fewer frames for a fast
  preview (rendering frames is the slow part, not the mux). Frame `dt` + camera sway scale with it so
  timing/motion stay constant; `record.sh` muxes at the same fps so duration + audio sync hold.

### Music (data-driven score files, no recompile)
- **Streaming synth**: the track renders in time-ordered segments on a background thread, so live
  playback + the show start together ~1 s after launch (the producer races ahead at ≈7× realtime)
  instead of waiting for the whole render — no more dead black screen, and `@@` anchors stay
  sample-locked. The streaming engine matches the batch render within ~1 LSB (verified). The loader
  covers the brief lead-in. `MARTIN_MUSIC=<wav>` (pre-rendered, what the bundle ships) skips the
  render; `MARTIN_STREAM_WAV` dumps the streamed render for A/B debugging.
- Multi-core batch synth render (~2× faster, deterministic) for recordings + the bundle WAV.
- Tracker DSL: sections/phases, per-section chords, multi-bar melody/arp/bass note-lanes, drum
  patterns, dynamics ramps, and free-form mix/fx `set` knobs.
- Per-section overrides: `<section>.set key=value` (knobs) and `<section>.fx: …` (which layers /
  transition accents fire — so a genre picks its own accents without abusing section names).
- Synth voices incl. a hardstyle kick, Reese/woozy bass, singing 5-saw lead, supersaw+choir wall,
  classic M1 "house organ", donk, casio; 2-band master, glue comp, diffuse reverb (+ section depth
  automation), Haas widening, sidechain, atmosphere bed; optional 2× oversampling (`set oversample=1`).
- Structural lint of the score with `MARTIN_SCORE_STRICT=1` to make warnings fatal.
- Example scores showing the range: `assets/tropical.txt`, `assets/rain.txt`.
- Score split: the engine ships a **neutral** tropical-house builtin (`assets/score.txt`); each
  production owns its own arrangement (e.g. `productions/camping/score.txt`, the "Op de Camping" track).

### Content & productions
- The **default show is the intro production** — a bare `cargo run` (and a fresh `git clone`) plays
  the same showcase CI bundles into the single binary. Its procedural splats are synthesized by
  `build.rs` (via `build/gen_splats.rs`, all 11 shapes) if missing, so a clean checkout builds + runs
  with no python/numpy step; CI dropped its pip/numpy generate step and the old
  `pipeline/gen-demo-splats.py` was removed. The older effect-catalogue demo stays at `assets/demo.show`.
- `productions/` — one folder per demo (showbook + `.show` + bundle recipe). **intro**: the
  licence-cleared, repo-only showcase CI bakes into the single-binary. **camping**: the full-fat
  "Op de Camping" demo (designed showbook-first; uses the big local captures, stand-ins until shot).
- BornHack host-camp logo (`assets/bornhack.{svg,glb,dae}`): wordmark from the bornhack-website repo,
  extruded via `pipeline/svg_import.py`; used in the camping show as a `glb:` venue dissolve + an
  `svg:` outro greeting. **BSD-3-Clause © BornHack.**
- `bitterbal.glb` — the Maali bitterbal as glTF (`pipeline/bitterbal_glb.py`): 5 MB vs 19 MB obj,
  carries vertex colours; the shows sample it instead of the .obj.

### Tooling / CI
- CI: rustfmt, clippy (`-D warnings`), cross-platform build+test, REUSE, advanced CodeQL, cargo-audit.
- Dependabot (weekly) with auto-merge of green patch/minor bumps; `main` branch protection.
