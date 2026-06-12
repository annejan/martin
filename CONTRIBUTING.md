<!--
SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
SPDX-License-Identifier: MIT
-->

# Contributing to martin

martin is a CUDA-free Bevy + Vulkan 3D-Gaussian-splat morphing/compositing demo engine (a deFEEST
production). Patches welcome. This file is the short version; see **README.md** (overview),
**USAGE.md** (every `MARTIN_*` knob + the `.show` format) and **DESIGN.md** (architecture).

## Build & run

A **nightly** toolchain is required — `bevy_gaussian_splatting`'s default features use
`nightly_generic_alias` (GATs). `rust-toolchain.toml` selects the `nightly` channel; it is
deliberately **not pinned to a date** (we ride current nightly).

```bash
# Linux build deps (udev/alsa for bevy_gaussian_splatting, wayland/xkbcommon for winit):
sudo apt-get install -y libudev-dev libasound2-dev libwayland-dev libxkbcommon-dev

cargo +nightly run --release          # the default demo
cargo +nightly test --release         # the unit tests (parsers, timeline, score, effects — no GPU)
./record.sh out.mp4                    # render the whole timeline to an mp4 (headless)
```

## Before you push

CI (`.github/workflows/`) gates pull requests on these — run them locally first:

```bash
cargo +nightly fmt --all --check       # rustfmt (nightly — rustfmt.toml uses unstable options)
cargo +nightly clippy --all-targets -- -D warnings
cargo +nightly test --release
```

- **REUSE / licensing:** every file needs an SPDX header or a `REUSE.toml` entry (`reuse lint`).
- **CodeQL** + **cargo-audit** run too; **Dependabot** auto-merges green patch/minor dep bumps.
- `main` has branch protection (required checks) but the owner can still push directly.

## The music is data, not code

The track is `assets/score.txt` — a tracker DSL parsed by `src/score.rs`, synthesised by
`src/audio.rs`. Edit the score (or `MARTIN_SCORE=<file>`) and re-render; **no recompile**. The loader
lints the score — run with `MARTIN_SCORE_STRICT=1` to make a phase/bar typo fatal. See the comments at
the top of `assets/score.txt`, plus `assets/tropical.txt` / `assets/rain.txt` for the range.

## The splat-renderer fork

`bevy_gaussian_splatting` is patched to our fork (the `martin` branch of
`annejan/bevy_gaussian_splatting`) via `[patch.crates-io]` in `Cargo.toml`; `Cargo.lock` pins the
exact commit. Keep edits **minimal, gated, and documented** (see the branch's `CHANGES.md`) so they
stay easy to rebase onto upstream and to submit back as a PR later. Edit shaders by committing to the
branch and `cargo update -p bevy_gaussian_splatting`; for heavy local iteration, temporarily point the
patch at a checkout (`path = "../bgs-fork"`).

## SH build profiles (sh0 / sh3)

Spherical-harmonic degree is a **one-hot compile-time crate feature**, so it can't be a runtime switch.
martin exposes it as two profiles (`Cargo.toml [features]`): **`sh0`** (default — flat colour, lean) and
**`sh3`** (degree-3 view-dependent colour, for real captures). Build either with the `.cargo/config.toml`
aliases — `cargo b-sh3` targets `target/sh3/` so the sh0 and sh3 binaries coexist (switching is a disk
cost, not a recompile):

```bash
cargo b-sh0 / cargo r-sh0     # sh0 (default) — plain target/release/
cargo b-sh3 / cargo r-sh3     # sh3           — target/sh3/release/
```

Because the two are mutually exclusive, sh3 needs `--no-default-features` (the alias handles it). Bundled
sh3: `cargo build --release --no-default-features --features sh3,bundle`. Synthetic content (text/morph)
renders identically in both — verify any SH change against a real capture, not just the demo.

## Commits

Conventional-ish, imperative subject (`area: what`), a body that says *why*. Keep diffs coherent;
update `CHANGELOG.md` for anything user-facing.
