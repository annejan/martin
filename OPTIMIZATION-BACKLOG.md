<!--
SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
SPDX-License-Identifier: MIT
-->

# Optimization backlog

Verified-but-not-yet-done optimizations from the 2026-06-26 hunt (each was checked against the real
code — hallucinations dropped). The easy/safe wins already shipped; what's left is **delicate** and
each deserves its own focused session with the right verification. Measured perf model is in `PERF.md`.

## Already shipped (don't redo)

Splat streaming · free-source-after-build · async/off-thread reel build + rayon sampling ·
`MARTIN_COUNT_SCALE` + `MARTIN_QUALITY` scaling compose · count: hygiene across all shows · synth-WAV
cache · causal FFT analyser · cine post-FX · camera spline · FPS HUD · perf knobs (SPLAT_SCALE /
SORT_BITS / QUALITY / VSYNC / BLOOM / fullscreen) · fork §9 quad-cut (sh0) · Bevy 0.19. **This round:**
trimmed splat features (−51 crates) · `b-fast` no-LTO profile · parallel `smoke-shows.py` · sh0 sort
default 24-bit · compose `disk:`/`aniso:` honoured · `--validate` peak-resident heads-up · `~fade` on
the opacity path (no doubled cloud). **2026-07-01:** #6 batch-synth rebalance (below) — ~30% faster
score render (~3.6s → ~2.5s on the builtin score), verified byte-identical.

## Remaining — delicate, needs care

### ~~#6 — Rebalance the batch synth into even chunks~~ — SHIPPED 2026-07-01
`stream.rs`'s parallel batch render now chunks `L_WALL` (the single fattest lane — 12 supersaw/choir
voices per bar-event) into `min(rayon::current_num_threads(), lane.len())` pieces, renders each into
its own zeroed buffer in parallel, and sums the results — an EXACT reconstruction, not an approximation:
`render_into` (audio/mod.rs) writes exactly `dur` samples per event with a hard boundary (the 4ms
release fade is *inside* that window, no tail beyond it), so L_WALL's bar-quantized events never touch
the same sample index — at every index at most one chunk is non-zero, so summing is `0.0 + x == x`,
bit-exact. Verified with the required WAV byte-diff: a fresh isolated-worktree rebuild of the pre-change
commit vs the new code produces the identical builtin-score WAV md5 (`b417cf46b6f9f411642cf312f9d5167d`
— the same hash this session's earlier tempo/grid/swing work already established as the byte-identity
baseline). Measured ~3.6s → ~2.5s (3 runs each) on the builtin score.txt. Scoped to L_WALL only (the
diagnosed bottleneck) rather than generalized to every lane, to keep the change small and the
correctness argument airtight — a lane whose events *could* overlap would need the reorder-to-epsilon
argument instead, not this exact one.

### #7 — `par_iter` the compose-stage build  ·  K-core startup-build win (live only)
`compose.rs` build loop. The reel got per-object parallelism; compose samples/normalizes/resamples one
object at a time. **Risk:** the loop interleaves `commands.spawn` (main-thread) + glTF-scene spawns with
the CPU sampling, and `sample_content` borrows `&assets`/`&state` — needs a sample→spawn split + `Send`
plumbing. Startup wall-clock only (record/shot build inline; live windowed loads benefit).

### Fork-shader batch (`../bgs-fork`, branch `martin-tightcut`)  ·  the full clone→edit→A/B→push→repoint dance
Confirm the local checkout matches the pinned commit (`Cargo.lock` rev `608c7d17`); then path-patch
martin to `../bgs-fork`, edit, rebuild **sh0 AND sh3**, A/B both, push the `martin-tightcut` branch,
repoint the git dep + `cargo build` (re-resolves the lock to the new rev).
- **#5 — dead-discard + alpha early-out** (`gaussian.wgsl:705-707` `dist²>9` never fires; ~21 % of every
  quad is sub-1 %-alpha fill). Clean but **modest** (saves shading, not rasterization).
- ~~**#4 — move the PNG dump off-thread**~~ **TRIED 2026-06-26 → NO-OP, dropped.** Built it (martin-side,
  `AsyncComputeTaskPool` + atomic tmp→rename) and A/B'd a full 720p record: **109 s sync vs 108 s
  off-thread = identical.** The record is NOT encode-bound. The "~29× headroom" was a mirage — the
  "712 fps ceiling" is the **CPU-submit `MARTIN_BENCH` number, which over-reports ~20×**; the real
  headless record renders at ~17 fps because the **headless `ScheduleRunner` doesn't pipeline a render
  thread** (CPU-schedule-bound, exactly what `--benchmark` measured: ~9–18 fps headless vs ~40 windowed).
  The ~15 ms PNG encode hides behind the ~60 ms render, so off-threading it saves nothing. **The only
  real record speedup is enabling pipelined rendering in the headless loop** (the windowed app already
  pipelines → ~2×, not 30×), or rendering windowed-focused — both deeper + riskier than they're worth
  for an offline master.
- **#8a — SH-gate the fragment `sigma`** (`gaussian.wgsl:714` hardcodes `1/3`; §9 shrank the vertex quad
  to 2.4σ on sh0 but the fragment still shades as 3σ → sh0 splats render ~20 % tighter than a true
  Gaussian). **NOT a clear win — it changes the look of ALL sh0 content** (which the demos are tuned on).
  Do only if deliberately re-baking the splat falloff.

## Marginal (idle-only)

Backdrop-overdraw `--validate` heuristic (flag large far-Z props lacking `dscale:`) · slim
`match_reorder` to copy pos+col only (24 B) on the `MARTIN_PAIR=match` path · pipeline the serial synth
finisher chain (~0.5 s, only after #6) · warm persistent renderer for single-show probe loops.

## Bigger picture (not perf)

The engine is **fast + robust + live-playable** (`--quality low`/`potato` clear 30/60 fps). The
critical path for Evoke isn't more perf — it's the **demo itself**: one coherent show that carries a
story, built on the now-strong engine. Design it in a `SHOWBOOK` before rendering.
