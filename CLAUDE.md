# CLAUDE.md

Guidance for AI assistants (Claude Code and friends) working in this repository. It orients you fast,
points at the authoritative docs, and records the conventions CI enforces. When this file and a
deeper doc disagree, the deeper doc wins — keep this one short and link out.

> Companion file: **`AGENTS.md`** is the *demoscene-authoring* brief (the "spend the CPU, dazzle the
> eye" philosophy, the score/audio tuning, the Blender↔martin bridge). This file is the *engineering*
> orientation: layout, build/test workflows, and the rules that gate a merge. Read both.

## What martin is

A music-synced **Gaussian-splat demoscene engine** built on **Bevy 0.18 + `bevy_gaussian_splatting`
7.0.2**, rendering through **wgpu → Vulkan** with **no CUDA/ROCm**. It flies a camera around 3D
splats while they **morph into one another** (per-Gaussian, on the GPU), with HDR bloom on black,
all driven by a synth track. A show is a **`.show` file** (run it: `martin <show>` / `--production
<name>`); it expands into `MARTIN_*` env vars — the internal IR — which a **CLI** layers run-mode flags
on top of (`--record`/`--shot`/`--validate`/…). No separate config file.

Two ways to stage content (both in `src/scene/`):
- **Reel / morph timeline** (`scene::sequence`, `MARTIN_SEQ`) — a chain of *parts* that each assemble
  out of a ball cloud and morph into the next.
- **Stage / composition** (`scene::compose`, `MARTIN_COMPOSE`) — many objects on one stage at once,
  placed and animated, camera flowing among them.

## The docs map (read the right one)

| Doc | What it covers |
|---|---|
| `README.md` | Project overview, the splat-creation pipeline, build profiles. Start here. |
| `DOMAIN.md` | The domain model + canonical vocabulary (Reel, Stage, Shot, Score, Show, anchor/cue, Showbook). |
| `USAGE.md` | **The CLI + `MARTIN_*` env reference** + the `.show` file format. The single source of truth for knobs. |
| `DESIGN.md` | Engine architecture, design decisions, the one deliberate fork edit, refactor debt. |
| `CONTRIBUTING.md` | Build deps, the CI gates, the SH profiles, commit style. |
| `AGENTS.md` | Demoscene authoring + audio/score tuning + the Blender bridge + common pitfalls. |
| `ART-DIRECTION.md` | How to shoot/prep good splats and the **scene design language** (read before composing scenes). |
| `SHADER-BLUEPRINT.md` | The per-particle / WGSL transition shader work. |
| `productions/README.md` | The engine-vs-production split; one folder per demo. |
| `pipeline/AERIAL-CITIES.md` | The aerial-city capture pipeline (gitignored, ©Google constraints). |
| `CHANGELOG.md` | Update this for anything user-facing. |

## Repository layout

```
src/
  main.rs            wiring only — registers plugins (Camera/Scene/Capture/Music/Loader)
  scene/             the two staging modes + shared content
    sequence/        the morph timeline (parse/model/build/director)
    compose.rs       the composition stage
    content.rs       PartContent — what a part actually is (text/ply/mesh/shader/…)
    beat.rs caption.rs colorize.rs effects.rs spectrum.rs gl_dissolve.rs shader_part.rs
  audio/             FunDSP synth: voices.rs effects.rs render.rs stream.rs analyze.rs
  score/             tracker-DSL parser: types.rs parse.rs validate.rs dump.rs
  music.rs           Bevy plugin binding score ↔ audio playback
  morph.rs           per-Gaussian pairing (Morton rank / nearest-match) — why a morph flows or balls
  camera.rs          orbit camera, FOV, waypoints/fly, beat-pump
  capture.rs         headless PNG recording (MARTIN_RECORD)
  loader.rs          splat / asset loading
  mesh.rs glb.rs splat_image.rs text.rs   content → gaussians (mesh/glTF/PNG-SVG/glyphs)
  splatgen.rs bin/splatgen.rs              procedural cloud generator (shared with build.rs)
  background.rs post.rs particles.rs       backdrop shader / post FX / particle layer
  show.rs sync.rs waypoints.rs serve.rs mcp.rs envvar.rs validate.rs bundle.rs fourd.rs
assets/      shipped art, fonts, shaders (bg.wgsl/post.wgsl), score.txt, .show examples
productions/ one folder per demo (intro, camping, austin/nyc/cities, …) — theme-specific content
parts/       reusable, tested building blocks (sequences/signatures)
pipeline/    CUDA-free splat-creation + tooling scripts (python/bash), incl. show_layout.py, blender_bridge.py
build.rs     synthesizes the demo's gitignored .ply at build time; bundles assets for --features bundle
```

## Build, run, test

**Nightly toolchain is required** (`bevy_gaussian_splatting` default features use GATs behind
`nightly_generic_alias`). `rust-toolchain.toml` pins the `nightly` channel (unpinned date — rides
current nightly). Linux build deps: `libudev-dev libasound2-dev libwayland-dev libxkbcommon-dev`.

```bash
cargo run --release                              # the default demo (windowed)
cargo run --release -- <show.show>               # play a .show  (or --production <name>)
cargo run --release -- <show.show> --validate    # dry-run: print the resolved timeline + exit
cargo run --release -- --synth-wav out.wav       # render the synth to WAV, then exit (headless)
cargo test --release                             # 100+ pure unit tests (parsers, timeline, score, effects — NO GPU)
./record.sh out.mp4                              # render the whole timeline to mp4 (headless PNG dump → ffmpeg)
```

A run is a **CLI**: `martin [SHOW] [--record/--shot/--shots/--bench/--validate/--serve/--synth-wav/
--production NAME]`, plus `martin mcp`. Each flag compiles to its `MARTIN_*` env var with overwrite, so
the precedence is **CLI flag > env > `.show` [settings] > default** (env still works everywhere). See
`USAGE.md`. `cargo run` defaults to the `martin` binary; the procedural generator is the second binary
(`cargo run --bin splatgen -- list`).

### SH build profiles (sh0 / sh3)

Spherical-harmonic degree is a **one-hot compile-time crate feature** (not a runtime switch). Use the
`.cargo/config.toml` aliases — sh3 targets a separate dir so both binaries coexist:

```bash
cargo b-sh0 / cargo r-sh0     # sh0 (default) — flat colour, lean → target/release/
cargo b-sh3 / cargo r-sh3     # sh3 — degree-3 view-dependent colour (real captures) → target/sh3/release/
```

Synthetic content (text/morph) renders identically in both; verify any SH change against a real
capture. Bundled single binary: `cargo build --release --features bundle` (sh3 bundle needs
`--no-default-features --features sh3,bundle`).

## CI gates — run these before you push

CI (`.github/workflows/build.yml`) gates PRs on these; run them locally first (all on nightly):

```bash
cargo fmt --all --check                   # rustfmt — rustfmt.toml uses UNSTABLE options, needs nightly
cargo clippy --all-targets -- -D warnings # warnings are errors; intentional lints are #[allow]ed
cargo test --release
reuse lint                                # every file needs an SPDX header or a REUSE.toml entry
```

Also in CI: a cross-platform build/test matrix (Linux/Windows/macOS), the `--features bundle`
single-binary build of the **intro** production (with a strict score-validate step), **CodeQL**, and
**cargo-audit**. **Dependabot** auto-merges green patch/minor dep bumps. `main` has branch protection.

## Conventions

- **The music is data, not code.** The track is `assets/score.txt` — a tracker DSL parsed by
  `src/score/`, synthesised by `src/audio/`. Edit the score (or `MARTIN_SCORE=<file>`) and re-render;
  **no recompile**. Run with `MARTIN_SCORE_STRICT=1` to make a phase/bar typo fatal. The melodic lines
  (lead/arp/bass) come from a real transcription — **do not change the note data**; only drums,
  dynamics, spatiality, and camera are free.
- **Engine vs production split.** The engine (`src/`) stays theme-agnostic; theme-specific content
  lives in `productions/<name>/`. The test for new work: scene blocks / density ramps / camera regimes
  → engine; a specific demo's scenes/captures → that production's folder; reusable tested blocks →
  `parts/`. See `productions/README.md`.
- **The splat-renderer fork.** `bevy_gaussian_splatting` is patched to our fork (the `martin` branch
  of `annejan/bevy_gaussian_splatting`) via `[patch.crates-io]` in `Cargo.toml`; `Cargo.lock` pins the
  exact commit. Keep fork edits **minimal, gated, and documented** (the branch's `CHANGES.md`). Edit
  shaders by committing to the branch + `cargo update -p bevy_gaussian_splatting`; for heavy local
  iteration, point the patch at a checkout (`path = "../bgs-fork"`).
- **REUSE / licensing.** Every file needs an SPDX header **or** a `REUSE.toml` entry (MIT, ©Anne Jan
  Brouwer). Root markdown docs are licensed via the `REUSE.toml` path list — **add new docs there**
  (this file is already listed). `reuse lint` must pass.
- **Commits.** Conventional-ish, imperative subject (`area: what`), a body that says *why*. Keep diffs
  coherent. Update `CHANGELOG.md` for anything user-facing.
- **New `MARTIN_*` env var or `.show` token?** Document it in `USAGE.md` — that file is the knob
  reference and authors rely on it.

## Common pitfalls (engine-side)

- **Props/splats vanish at steep angles** — the backdrop cloud sorts over them (per-cloud depth sort,
  no cross-cloud z-buffer). Push the backdrop centre back. See `ART-DIRECTION.md` § Z.
- **Recording overflows `/tmp`** — a full 60 fps dump is ~5300 PNGs (~10 GB) and overflows the
  RAM-backed tmpfs. Point scratch at real disk: `TMPDIR=/path ./record.sh out.mp4`. Use
  `MARTIN_PREVIEW_FPS=8` + a small `MARTIN_MORPH_COUNT` for fast/small previews.
- **Spawning multiple renders at once wedges the GPU** — render one at a time, small budget; iterate
  with `pipeline/show_layout.py` (GPU-free) before rendering.
- **Score edits** — changing phase bars without adding matching `pN` drum patterns yields silence; the
  phases-sum + 1 fill must equal the section's bar count. See `AGENTS.md` § Common Pitfalls.
- The GPU is the bottleneck (AMD Radeon 860M iGPU dev box); CPU/RAM are abundant. Spend cores on
  precompute freely; be thoughtful about splat overdraw / fill rate. See `AGENTS.md` § Philosophy.
