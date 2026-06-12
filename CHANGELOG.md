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
- SH build profiles: `sh0` (default, flat colour) and `sh3` (degree-3 view-dependent glint, for real
  captures) — `cargo b-sh3` builds into a separate target dir so both binaries coexist.

### Music (data-driven — `assets/score.txt`, no recompile)
- Tracker DSL: sections/phases, per-section chords, multi-bar melody/arp/bass note-lanes, drum
  patterns, dynamics ramps, and free-form mix/fx `set` knobs.
- Per-section overrides: `<section>.set key=value` (knobs) and `<section>.fx: …` (which layers /
  transition accents fire — so a genre picks its own accents without abusing section names).
- Synth voices incl. a hardstyle kick, Reese/woozy bass, singing 5-saw lead, supersaw+choir wall,
  classic M1 "house organ", donk, casio; 2-band master, glue comp, diffuse reverb (+ section depth
  automation), Haas widening, sidechain, atmosphere bed; optional 2× oversampling (`set oversample=1`).
- Structural lint of the score with `MARTIN_SCORE_STRICT=1` to make warnings fatal.
- Example scores showing the range: `assets/tropical.txt`, `assets/rain.txt`.

### Tooling / CI
- CI: rustfmt, clippy (`-D warnings`), cross-platform build+test, REUSE, advanced CodeQL, cargo-audit.
- Dependabot (weekly) with auto-merge of green patch/minor bumps; `main` branch protection.
