# Code Quality Audit — `src/` (2026-06-15)

Thermo-nuclear maintainability review. Scope: martin engine `src/` only — the
`splat-tools/brush` vendored fork is excluded. Findings ordered by structural
severity. The code is correct and well-commented throughout; every finding is
about *complexity-spreading* shapes where one tweak now demands synchronized
edits in two places.

---

> **STATUS (2026-06-16):** ALL RESOLVED — #1 ✅ · #2 ✅ · #3 ✅ · #4 ✅ · #5a ✅ · #5b ✅ · #6 ✅.
> #1: `synth_track` is now a thin wrapper over `stream::produce`; the entire mirror DSP (render.rs
> passes + master, effects.rs whole-buffer fx) is deleted (~880 lines, render.rs 1166 → 484). One
> engine — a recording is byte-for-byte the streamed signal (`batch_is_the_collected_stream`),
> verified ≤1 PCM LSB vs the old batch on the real score.

## 🔴 BLOCKER 1 — `audio/render.rs` (1166 lines): `collect_events` is a full second copy of the batch passes

Dominant structural regression, self-documented as debt (render.rs:760–767,
stream.rs:8–12: *"a mirror of the batch pass functions… Once the stream is
trusted by ear, batch can be switched onto `produce` too and the duplication
removed"*).

The entire score→placement mapping — *for every note: time, amp, pan, seed,
duration, voice, fx-gate* — is written **twice**:

| Logic | Batch | Mirror |
|---|---|---|
| snare pan `match i%3 {0=>-0.2,1=>0.15,_=>-0.05}` | L115 | L835 |
| hat pan `i%2 ? 0.65 : -0.65` | L130 | L847 |
| seeds `0x55/0x77/0x6E/0x1A/0x2B/0xB5/0xD0/0x40/0x4C` | drums/voices | collect_events |
| intro percussion bars `2..build`, the `b>=4/5/6` ladder | L52–77 | L803–832 |
| wall's 4× `render_into` (saw L/R + choir L/R) | L322–343 | L970–975 |
| FX gating tree (riser/jet/impact/bang per section) | L488–547 | L1069–1163 |

Every constant duplicated verbatim. Kept in sync **by ear** + one
`stream_matches_batch` test. Every future tweak is a two-place edit whose
failure mode is silent drift.

**Code-judo move (already named in the comments):** make the lane definitions
the single source of truth. `collect_events` already produces
`[Vec<Event>; LANES]` of `(t, RenderFn)`. The batch passes should **consume that
same list** — run each lane's closures into its buffer in time order, then keep
the existing thread-parallel per-lane sum. `render_drums` / `render_voices` /
`render_harmony` / `render_fx` collapse to "run lanes 0 / 1–4 / 5–8 / 9", and
~390 lines of mirror logic delete. Byte-for-byte batch output is preserved
because the closures *are* the batch render fragments.

File also crosses the 1000-line smell line; unifying drops it well under.

## 🔴 BLOCKER 2 ✅ DONE — `compose.rs` ↔ `sequence/build.rs`: "build a placed cloud" pipeline copy-pasted across modules

> Fixed: `scene::content::sample_content(content, entrance, …)` is the single shared
> shaper (text/pen-write/`part_gaussians` + normalize/resample); both `compose` and
> `build` call it. The `*0.5` vs `content_radius` "drift" was confirmed correct per
> each call's normalization context, not a bug.


Both files independently run the identical pipeline:

```
wait-for-all-loads guard
 → PenWrite-text special case (same MARTIN_PW_STEP / MARTIN_PW_SPLAT / build_text_penwrite_gaussians)
 → part_gaussians
 → normalize_to(NORMALIZE_EXTENT)
 → resample_morton(count)
 → tint apply (crate::scene::colorize::apply)
 → spawn GaussianInterpolate if transition else plain handle (same CloudSettings, same source_cloud fallback)
 → frame camera from placed centres/radius
```

compose.rs:335–405 vs build.rs:93–248. **Already drifted**: compose sizes the
transition source at `NORMALIZE_EXTENT * 0.5`, build uses `content_radius`.

**Fix:** extract `fn build_placed_cloud(content, transition, tint, rot, count,
&mut assets) -> (shaped, Option<source>)` + a shared spawn helper. Both call
sites shrink to a loop body; shaping/tinting/spawn rules can't drift.

## 🟠 MAJOR 3 ✅ DONE — modifier-token parser duplicated (`parse.rs` ↔ `compose.rs`)

> Fixed: `effects::parse_fx_modifier(tok) -> Option<Result<FxMod, String>>` (typed
> `FxMod::{Entrance,Deform,Tint}`) is the shared `~`/`^[:amp]`/`tint:` parser; both
> `parse_seq` and `parse_compose` fall back to it. Tested.


`~transition`, `^name` / `^name:amp`, `tint:` parsed with the same
`strip_prefix → match → eprintln warn` blocks in **both** `parse_seq`
(parse.rs:104–176) and `parse_compose` (compose.rs:166–200). `parse_seq` handles
~11 sigils, `parse_compose` a hand-copied 3-sigil subset.

**Fix:** one `parse_modifier(tok) -> Option<Modifier>` (typed enum) consumed by
both filters. A new tint mode / `^` syntax change becomes a one-place edit.

## 🟠 MAJOR 4 ✅ DONE — `parse.rs`: `Shot` constructed 3× with all 16 fields spelled out

> Fixed: `Shot::base(content)` sets every default (hold 1.5, morph 3.0, bulge 0.9,
> rest `None`); the legacy literals are now `..Shot::base(content)` overrides. New
> fields stop rippling. Tested (`shot_base_is_all_defaults`).


`Shot { content, hold, morph, bulge, transition: None, anchor: None, … }` appears
verbatim three times (parse_seq L203, MARTIN_TEXT L263, MARTIN_PLY L302/L321),
each a 16-line mostly-`None` literal. Struct has grown to 16 fields
(model.rs:17–34) → **every new field is a 4-place edit**.

**Fix:** `impl Default for Shot` (or `Shot::new(content)`), set only the
non-defaults. New fields stop rippling.

## 🟡 MINOR 5 ✅ DONE (5a+5b; split deferred) — `morph.rs`: cohesive, but two extractions + a split are due

> Fixed 5a: `fn bounds(&[Gaussian3d]) -> ([f32;3],[f32;3])` replaces the 4 bbox loops.
> Fixed 5b: `reposition(shape, |k,p|->pos)` + `depart(…)` (= reposition + fade) own the
> shared `*_of` skeleton; 14 builders shrank to just their `place()` math, byte-identical
> (ball/helix/condense keep hand loops — they also write visibility/scale). The file split
> (`morph/{arrival,departure,sample}.rs`) is deferred to next growth, as the review allowed.


Healthiest big file — pure, well-tested transform library, not spaghetti. Two
reductions:

- **Bbox loop copy-pasted 5×.** `lo=[MAX;3]; hi=[MIN;3]; for g { min/max }`
  recurs at resample_morton:103, match_reorder:142, flatten_of:292, extent_of:639
  (+ normalize's variant). Extract `fn bounds(&[Gaussian3d]) -> ([f32;3],[f32;3])`.
- **~13 `*_of` functions share one skeleton:**
  `shape.iter().enumerate().map(|(idx,g)| { let k = idx as u32; … }).collect()`.
  A `fn reposition(shape, faded, |k,p| -> [f32;3])` halves each
  (ball/explode/implode/drop/rain/funnel/condense/shatter/wash/disperse/evaporate/sink).

Then split along the three existing concerns: `morph/{arrival, departure,
sample}.rs`. Do it at next growth; not a blocker today.

## 🟡 MINOR 6 ✅ DONE — `build.rs:356–384`: fragile parallel-Vec collapse

> Fixed: `BuiltShot`s are now pushed directly in the per-part loop (one pass); the
> `shapes`/`sources`/`out_clouds` side-Vecs + the `.next().expect()` zip-drain are gone.


`BuiltShot` assembly drains three iterators with `.next().expect(...)` while
zipping a 5-deep tuple `((((shot,transition),deform),raster),start)`. The
`BuiltShot` consolidation is the right direction (model.rs:48). Fold
`shapes`/`sources`/`out_clouds` into the same `zip` chain, or push `BuiltShot`s
directly in the earlier per-part loop (189–248).

---

## Verdict: CHANGES REQUESTED

Two structural blockers (#1, #2) + two cross-module duplications (#3, #4). None
are "it doesn't work" — all four are complexity-spreading shapes where one tweak
demands synchronized edits, and the codebase already shows drift from exactly
that.

**Priority:** #1 first (biggest payoff, pre-authorized by its own comments,
deletes ~390 lines, unifies the batch/stream split), then #2/#3/#4 as a
"deduplicate the show-building + parsing layer" pass. #5/#6 are next-touch
cleanups.

**Keep:** morph.rs purity + test coverage; the cue-timeline functions in
model.rs (small, pure, shared); the `BuiltShot` consolidation — all the right
kind of structure.
