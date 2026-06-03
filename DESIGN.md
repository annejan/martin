# martin — show-engine design (DRAFT, for discussion / keep options open)

> **Status:** DRAFT. A design exploration, co-written by **annejan** and **Kloot**
> (deFEEST), to be read and argued over in the morning. It recommends directions
> but **locks nothing in**. Every section gives a leaning *and* the alternatives,
> with honest effort/risk, and each leaning is tagged `(→ OQ#n)` to the matching
> open question so a reader can jump straight to the fork. This is a **worksheet,
> not a verdict.**
>
> Where it cites the engine it means the engine **as it actually is today** — the
> file/line references below were checked against the current `src/` and the
> vendored crate, and the doc was *re-based onto that live code* (the previous
> draft was written against a stale brief; see the reconciliation note in §0).
> The WGSL/Rust sketches fit *that* engine.
>
> Layers below this doc: `README.md` (what it is), `USAGE.md` (how to drive it),
> `ART-DIRECTION.md` (the look). This doc is the layer above: where the show
> *engine* goes next. The heavy reference material for the one deliberate fork
> edit lives in a companion file, `SHADER-BLUEPRINT.md` (only relevant once we
> decide to do it — see §5).

---

## 0. Decisions to make in the morning (read this first)

The whole point of this doc is to argue these out. The five highest-stakes forks,
surfaced up top; the full list of 15 is §9.2.

| # | Fork | Leaning (arguable) |
|---|---|---|
| **OQ#1** | **Grammar now** — extend the inline DSL (Option A) only, or jump to RON (Option B)? | A now, RON later — *unless* composition (§6) lands soon, which pulls RON's deps forward. |
| **OQ#4** | **Cue file format** — bars/beats + tempo-map, or pure seconds? 4/4-only for v1, or general meters? | Beats-first leaning, but pure-seconds is a real contender (§3.2 shows both). |
| **OQ#8** | **Composition backend** — one merged cloud always, or the hybrid (merge in-track, multi-entity across tracks)? Note: the hybrid **relaxes two of the six hard constraints** (§6.2). | Hybrid as the *ceiling*; one-merged-cloud as the safe v1. |
| **OQ#12** | **The shader fork (§5)** — do the one deliberate per-particle-phase edit now, or stay data-only? | Defer; data-only already covers a lot (and most of it already ships — see §0.1). |
| **OQ#13** | **Bundling unit** — one-binary-per-show (embedded default each), or one binary + loose `assets/shows/`? | Undecided. |

### 0.1 Reconciliation note — what already exists (the draft was re-based onto this)

A previous draft proposed a "named transition registry," a `~name` parser hook, and
~8 new `*_source` functions as *future work for tonight*. **Most of that already
ships.** Verified in the live tree:

- `Part` already carries `transition: Option<Transition>` (`main.rs:184`).
- A `Transition` enum already exists — `Morph, Ball, Fade, Explode, Implode, Drop,
  Swirl` — with `Transition::parse` and a `MARTIN_TRANSITION` global env default
  (`main.rs:151-175, 363`). **This enum *is* the "registry."** A new transition =
  one enum variant + one `morph.rs` fn + one `build_sequence` match arm; no new
  abstraction is needed.
- `parse_seq` already parses the trailing `~name` token (`main.rs:224-233`).
- `SeqState` already has `sources: Vec<Option<Handle>>` and
  `transitions: Vec<Transition>` (`main.rs:201-202`); `build_sequence` builds a
  per-part source cloud per transition (`main.rs:377-388`); `part_director`
  retargets `lhs` to that source and only pulses `bulge` for `Morph`
  (`main.rs:480-494`).
- `morph.rs` already implements `ball_of, fade_of, explode_of, implode_of,
  drop_of, swirl_of` (`morph.rs:54-165`) — the data-only transitions the old draft
  wanted to "write tonight" exist, under `*_of` names.

So the work split is **gap analysis, not greenfield.** §4 below is rewritten as
"shipped / gap," and the roadmap (§9.1) marks done steps as done. Concretely: do
**not** invent `fade_source`/`explode_source`/`wipe_source` — they would duplicate
the existing `*_of` functions and create exactly the "second drifting micro-API"
this doc warns against.

---

## 1. Where we are + design goals

### 1.1 The engine in 30 seconds

martin is a standalone Bevy 0.18 + `bevy_gaussian_splatting` 7.0.1 (vendored
fork). CUDA-free: wgpu → Vulkan / Mesa RADV on AMD. It flies an orbit camera around
3D Gaussian splats while they morph into one another.

The whole thing is **one timeline of parts** (`main.rs:151-192`):

```rust
enum Transition { Morph, Ball, Fade, Explode, Implode, Drop, Swirl } // already shipped
enum PartContent { Text(String), Splats(Vec<(String, Vec3)>) }
struct Part   { content: PartContent, hold: f32, morph: f32, bulge: f32,
                transition: Option<Transition> }   // transition already present
struct Sequence { parts: Vec<Part>, count: usize } // count = the ONE shared budget N
```

Authored via `parse_seq` (`main.rs:216`): a path-or-inline string, split on `;`/`\n`,
`#`-comments skipped, each part `head @ hold,morph,bulge` with an **optional trailing
`~name`** transition token (already parsed). `head` is `text:...` or
`splat:a.ply[+b.ply]`. Env shorthands (`MARTIN_PLY`, `MARTIN_TEXT`, `MARTIN_REFORM`,
`MARTIN_TRANSITION`, …) build `Part`s directly.

At build, every part is resampled to **one shared count `N`** (`resample_morton`),
part 0 assembles from a fuzzy ball (`ball_of`), and the show runs through **exactly
one** `GaussianInterpolate<Gaussian3d>` entity whose `lhs`/`rhs` handles are
retargeted per part by `part_director` (`main.rs:450`). Each non-`Morph` transition
builds its own *source cloud* (`fade_of`/`explode_of`/… in `build_sequence`,
`main.rs:377-388`) that the morph flies in from; `Morph` flows from the previous
part's shape with the `sin(pi*t)` ball-pulse keyed off `cs.bulge`. The morph is the
GPU compute blend `mix(lhs, rhs, t)`; `t` is a **single global** smoothstep-eased
scalar (`CloudSettings.time`). Recording (`record_driver`, `main.rs:543`) is
frame-indexed and deterministic.

### 1.2 The six hard constraints (carried verbatim — they shape everything)

These are ground truth. Every recommendation below stays inside them unless it
*explicitly* proposes a fork edit and says so.

1. **Toolchain/platform:** nightly Rust (pinned), Bevy 0.18, vendored crate via
   `[patch.crates-io]`. wgpu → Vulkan / Mesa RADV, AMD, **no CUDA / no ROCm**.
2. **One shared morph buffer → ONE count `N`.** Every part is `resample_morton`'d
   to the same `N`. Parts cannot have differing gaussian counts.
3. **Exactly ONE `GaussianInterpolate` entity** for the whole show, retargeted by
   swapping `lhs`/`rhs`. No multi-entity / layered compositing today.
4. **`CloudSettings.time` is one global per-frame scalar** applied identically to
   every particle. Any per-particle or staggered/time-offset effect **requires a
   shader change** in the vendored crate — there is no per-particle time/phase
   input today.
5. **Record mode must stay deterministic:** `record_driver` is frame-indexed
   (`clock.t = i*dt`, `dt=1/60`; yaw from frame index); `controls` /
   `advance_seq_clock` deliberately bail when recording. Any new runtime state
   must remain a pure function of frame index.
6. **`.ply` and `.mp4` are git-ignored** (multi-GB); only source/tools tracked.
   Loader rejects SuperSplat **compressed** PLY — uncompressed/standard PLY only.

### 1.3 Spirit (why the formats stay plain-text and CUDA-free)

martin is a demoscene production by **deFEEST**, co-written by **annejan** and the
AI under the scene handle **Kloot** (flat-Dutch for "Claude"). Two consequences:

- **Diffable, greppable, file-or-inline formats.** The show is a recipe in git, not
  a binary blob. `parse_seq` already auto-detects file-vs-inline; that property is
  sacred and every format below preserves it.
- **CUDA-free is pride, not just a constraint.** "made on AMD · Vulkan · Bevy" is a
  credit line (§7), so nothing here may quietly require CUDA/ROCm.

### 1.4 Design goals (the four gaps)

- **G1 — Music-timed sequencing.** Bind the timeline to a music master clock; author
  cues in bars/beats; keep record deterministic. (§3)
- **G2 — Pluggable transitions.** The ball-pulse is *one* transition of many. The
  `Transition` registry already exists; the gap is more data-only variants now and
  per-particle shader effects as a deliberate later edit. (§4, §5)
- **G3 — Scene composition.** A part should hold several elements — splat objects +
  splat-text + a logo image + (later) meshes — each with its own transform. (§6)
- **G4 — Single self-contained binary.** Binary + script + font embedded, with a
  loader screen; loose files stay the dev default. (§8)

These map onto §2–§8; §9 collects the genuine forks.

---

## 2. The sequencing language / the script

The data model is dead simple (`Vec<Part>` + one `count`). A format only has to
*produce that*. Don't out-build the model. Two facts constrain every option:

- **No embed/serde/RON crate exists today.** `Cargo.toml` pulls `bevy`,
  `bevy_gaussian_splatting`, and `ab_glyph` — no `serde`/`ron`/`image`. The *only*
  embedding precedent is `font.ttf` via `include_bytes!` (`text.rs`). The script is
  tiny (KB) so it is a real embed candidate; the multi-GB `.ply` are not
  (constraint #6).
- **`parse_seq`'s file-or-inline transparency must survive** any change.

The canonical example, in every variant below, is one real little show: *title →
egg → egg+dog side by side → reform into peace (punchy bulge) → tot ziens.* Today,
inline (note: `~name` already works):

```
# defeest opener
text:de FEEST              @2,3,0
splat:aegg.ply             @2.5,3,0
splat:aegg.ply+doggo.ply   @2,3,0.4
splat:martin-peace.ply     @2,3.5,1.2
text:tot ziens             @3,3,0
```

### 2.1 Option A — evolve the inline DSL (recommended for now) (→ OQ#1)

Layer onto the current splitter so **old scripts parse byte-identically**:

- **`key=value`** timing as an alternative to fragile positional floats: a token
  containing `=` → kv mode; else positional. Kills the `@,,1.2`-to-set-only-bulge
  footgun and the parse-default drift (positional defaults `1.5,3.0,0.9` at
  `main.rs:238` vs the hand-built parts' own values). **This is genuinely new
  work** (~30 lines; the existing parser is a forgiving `filter_map` at
  `main.rs:240`, so kv detection slots in cleanly).
- The **`~name` transition token already exists** (`main.rs:224-233`). The open
  spelling question is whether to keep bare `~name`, allow `~name(args)`, or move
  to a `kind=` kv field — the working spelling below is bare `~name` **pending
  OQ#2**; every §4 line uses it provisionally.
- **`cue NAME:` / `goto NAME:`** anchors — parsed and recorded, runtime deferred
  (→ OQ#3).

Same show (working spelling, mixing in the shipped transitions):

```
# defeest opener  (still a comment)
cue intro:  text:de FEEST            @hold=2 morph=3
            splat:aegg.ply           @2.5,3                      # positional still OK
            splat:aegg.ply+doggo.ply @hold=2 ~fade            # ~fade already ships
            splat:martin-peace.ply   @hold=2 morph=3.5 bulge=1.2 ~morph   # bulge=1.2 punch
goto intro: text:tot ziens           @3,3,0
```

**Pros:** smallest diff; `Part` already has `transition`; 100% backward compatible;
stays diffable; kills the positional footgun. **Cons:** bespoke micro-grammar, weak
errors, low ceiling once parts gain structure (per-part camera, full Vec3 offsets).
**Effort:** ~half a day for kv + cue tokens, no new deps. **Shorthand compat:**
untouched — `sequence_from_env` builds `Part`s directly and only calls `parse_seq`
on the `MARTIN_SEQ` branch.

### 2.2 Option B — structured file (RON), as a *second* input path (→ OQ#1)

A typed `parts` array via serde. **RON over TOML/YAML** because it is the native
Bevy/Rust idiom, represents Rust enums (`Text(...)`/`Splats([...])`, and the
existing `Transition` enum) directly, has no YAML footguns, and opens the door to
`#[derive(Asset)]` hot-reload (§2.5).

```ron
// assets/shows/defeest.ron   (assets/shows/ does NOT exist yet — to be created)
Show(
    count: 200000,
    parts: [
        ( content: Text("de FEEST"),                       hold: 2.0, morph: 3.0 ),
        ( content: Splats([("aegg.ply", (0,0,0))]),        hold: 2.5, morph: 3.0 ),
        ( content: Splats([("aegg.ply",(-1.2,0,0)),
                           ("doggo.ply",(1.2,0,0))]),       hold: 2.0, transition: Some(Fade) ),
        ( content: Splats([("martin-peace.ply",(0,0,0))]), hold: 2.0, morph: 3.5, bulge: 1.2 ),
        ( content: Text("tot ziens"),                      hold: 3.0, morph: 3.0 ),
    ],
)
```

The `transition:` field maps onto the *existing* `Transition` enum — RON gets it for
free. With `#[serde(default)]` you write only fields that deviate. **Pros:** real
types, real errors with line/column, one place for defaults, explicit per-splat Vec3
offsets, `count` in-file, room to grow (`camera:`, `rotation:`, per-element
`normalize:`). **Cons:** first non-trivial deps (`ron`+`serde`); verbose for a quick
test; two input paths to maintain. **Effort:** ~1 day. **Dispatch on extension:**
`*.ron` → RON, else → the extended `parse_seq`. The inline string stays forever
(tests + env shorthands).

### 2.3 Option C — bespoke block DSL (defer / probably never)

A purpose-built block grammar (hand-rolled or `winnow`/`pest`):

```
show defeest {
  count 200000
  text "de FEEST"          { hold 2; morph 3 }
  splat aegg.ply           { hold 2.5 }
  splat aegg.ply doggo.ply { hold 2; fade }
  splat martin-peace.ply   { hold 2; morph 3.5; bulge 1.2 }
  text "tot ziens"         { hold 3 }
}
```

**Pros:** best ergonomics, transitions as first-class verbs, full error control,
room for music markers (`at 4.0 { ... }`) and loops. **Cons:** most effort (~2–4
days) and *permanent* maintenance; reinvents what RON gives free; highest risk of
becoming a second drifting micro-language. **Verdict:** only build it if the show
language genuinely needs constructs RON expresses badly (music cues, conditionals,
loops) — a later phase, not now.

### 2.4 Sequence-file-as-asset / embedding (→ OQ#13)

This cuts across A/B/C, so treat it independently of syntax.

- **Where it lives:** `assets/shows/*.{seq,ron}` alongside `font.ttf`. **Note:
  `assets/shows/` does not exist yet — it is to be created.**
- **Mechanism — reuse the `font.ttf` precedent.** A built-in default ships as
  `static DEFAULT_SHOW: &str = include_str!("../assets/shows/default.seq");` (once
  that file exists). No new crate for one default. For a whole-dir embed,
  `rust-embed` — but for a single default, `include_str!` matches the codebase.
- **Resolution order (do this regardless of syntax):**
  1. `MARTIN_SEQ` is an existing file → load it.
  2. `MARTIN_SEQ` is a non-path string → parse inline (today's behaviour).
  3. `MARTIN_SEQ` unset and no shorthand env → **fall back to embedded
     `DEFAULT_SHOW`** instead of the current `aegg.ply` hardcode. This is what makes
     `./martin` self-contained: no env, no files, still plays a show.
- **The real gotcha — relative asset refs.** Script lines name `aegg.ply` as a
  *basename* resolved against `AssetPlugin.file_path` = `parent_dir(MARTIN_PLY)`. A
  bundled script has no `MARTIN_PLY`, so the root is unset and basenames won't
  resolve. Fix, cleanest first: let the show declare its own root (RON
  `assets_root: "assets"`, or a `#root: assets` directive in the DSL); or default
  the root to `assets/` when falling back to the embedded default.
- **Honest caveat (ties to §8):** "single binary" means *binary + script + font
  embedded; `.ply` loaded from an asset folder beside it.* The multi-GB `.ply` are
  git-ignored and won't be embedded. A truly self-contained demo needs at least one
  small splat shipped beside the exe (`luigi.ply` ~1 MB, `doggo.ply` ~13 MB) or
  `include_bytes!`'d. Do **not** promise a zero-file binary that references
  `bicycle.ply` (verified **1.52 GB** on disk).

### 2.5 `#[derive(Asset)]` hot-reload — what it actually takes (RON path)

The brief explicitly flags "sequence-file-as-asset," so sketch it rather than
hand-wave. A Bevy `AssetLoader` for a `.ron`/`.seq` show would:

- define `#[derive(Asset, TypePath)] struct Show { parts, count }` and an
  `AssetLoader` whose `load()` runs the same `parse_seq`/RON parse over the file
  bytes, producing the `Show` asset;
- have `setup` do `asset_server.load::<Show>("shows/default.ron")` instead of
  reading the string eagerly;
- gate `build_sequence` on the `Show` handle being loaded (it already gates on the
  `.ply` handles, so this is the same pattern).

**The catch for hot-reload:** `build_sequence` has a one-shot `state.built` latch
(`main.rs:306`) that never re-arms. Live editing the show requires detecting the
asset's `AssetEvent::Modified`, despawning the single interpolate entity, and
clearing `built` so the next `build_sequence` rebuilds. That re-build path is real
work, not free — list it as a RON-only nicety (→ OQ#1 colours how soon it's worth
it), not a v1 deliverable.

### 2.6 Recommended incremental path (options-open)

**Now:** Option A minimum slice — (1) `key=value` timing alongside positional;
(2) the embedded default + explicit resolution order + default asset root. The
`~name` token already exists; widen it (more variants / arg syntax) only after
OQ#2. Skip `cue/goto` runtime (parse-and-ignore is fine).

**Later:** add RON as a *second* path (dispatch on extension) when parts gain real
structure (per-part camera, explicit Vec3 offsets, music markers, composition — §6).
Keep the inline string forever.

**Defer:** Option C until the language needs what RON can't express.

No format is ever ripped out: inline stays (tests + env shorthands), RON is purely
additive, the embedded default makes the single binary real without touching the
giant-`.ply` story.

### 2.7 Demoscene prior art for the script/sequencing

- **Trackers (ProTracker/FastTracker, MOD/XM/IT) — rows-per-beat.** The bars/beats
  idea, with *fractional* `.beat`, is the tracker "row" model (rows quantize a beat;
  the playhead is a function of row × speed). The cue format in §3.2 is closer to a
  tracker row sheet than to anything new.
- **GNU Rocket** — the canonical sync tool; see §3 for the precise borrow/don't.
- **Bonzomatic / live-coding (Shader Showdown)** — the "edit-and-reload" loop; our
  `parse_seq`-re-run *is* the cheap version of this, which is why a live socket
  editor (Rocket-style) is deliberately out of scope.

---

## 3. Music-time and cue sheets

The single most important idea in demoscene sync — **GNU Rocket** — is: *the audio
playback position is the master clock; everything else is a pure function of song
time.* What to borrow precisely, and what to leave:

**Borrow from Rocket:** (1) audio position as master clock; (2) author in BPM/rows
but resolve to seconds at load; (3) the latency offset trick (Rocket's `+0.05`);
(4) smoothstep interpolation between keyframes — which martin *already* uses
(`factor*factor*(3-2*factor)`, `main.rs:490` = Rocket's "Smooth"). **Don't borrow:**
the live socket editor (overkill; `parse_seq` hot-edits cheaply by re-running), and
tracks-of-arbitrary-floats (constraints #3/#4 mean one entity, one global scalar —
martin needs *named time markers* that parts anchor to, a degenerate single-axis
Rocket). If transitions ever grow per-track float params, *then* a small set of
Rocket-style tracks earns its place — future, not now.

**Wider prior art for "audio drives everything":** **MilkDrop / projectM** and the
classic Winamp visualizers are the purest "audio position (and FFT) is the master
clock" precedent and are worth a nod alongside Rocket — they show how far a single
audio-driven playhead goes before you need per-element tracks.

### 3.1 Audio playback position as master clock — *and the dependency reality*

**Dependency check first (the doc is careful about this elsewhere, so be careful
here):** `Cargo.toml` depends on `bevy = "0.18"` with default features, which
**includes `bevy_audio`** (rodio-backed). So an audio stack *is* compiled in — but
**no audio is wired today**: there is no `AudioPlayer`/`AudioSink` spawn anywhere in
`src/`, and the clock is pure wall-time. Adding playback is net-new work, not a flip
of an existing switch. (If we ever want sample-accurate position we may add `rodio`
directly; note it as a dep cost, like §2 does for serde/RON.)

Today (`main.rs:498-510`) the live clock **accumulates delta-time**, which drifts
against the soundcard and never recovers from a stall:

```rust
if built { clock.t += time.delta_secs(); }   // drifts vs audio
```

Replace it, **live mode only**, with the music as master:

```rust
fn advance_seq_clock(rec: Res<RecordState>, state: Option<Res<SeqState>>,
                     audio: Res<MusicClock>, mut clock: ResMut<SeqClock>) {
    if rec.dir.is_some() { return; }                 // record mode drives clock itself (§3.4)
    if state.map(|s| s.built).unwrap_or(false) {
        clock.t = audio.position_secs() + AV_OFFSET; // master clock = the music
    }
}
```

**Bevy 0.18 mechanics.** `bevy_audio`'s `AudioSink` does *not* expose a
sample-accurate position. Two implementations behind one `position_secs()` seam
(→ OQ#5):

- **(A) Wall-clock anchored to audio start (recommended v1).** On `sink.play()`,
  record `start = Time::elapsed()`; `position_secs() = (Time::elapsed() - start)`. A
  *single* subtraction against one monotonic clock — self-heals after a stall.
  Residual drift = system-vs-soundcard skew (ppm, inaudible over 3 min). What most
  Bevy demos do.
- **(B) Sample-accurate via a custom rodio `Source` wrapper** publishing an
  `AtomicU64` sample counter; `position_secs() = samples / sample_rate`. The true
  Rocket/BASS approach, immune to skew, more code (+ a direct `rodio` dep). Ship (A);
  keep the `MusicClock` trait seam so (B) drops in without touching
  `advance_seq_clock`.

### 3.2 Cue-sheet file format — beats-first leaning, with the pure-seconds counter (→ OQ#4)

deFEEST's cues are *musical* (a drop lands on a downbeat, not at "37.214 s"). So the
**leaning** is beats-first with a tempo **map**, compiled to `(label → seconds)` at
load. **This is OQ#4, not settled** — the pure-seconds counter-sketch is shown right
after so the two can be compared, not just sold.

```
# martin.cues — from deFEEST, track "ascent.ogg"   [BEATS-FIRST — the leaning]
audio  = ascent.ogg
meter  = 4/4            # 4/4-only is itself OQ#4 — general meters are a real option, not a decree
offset = 0.35          # seconds of lead-in before bar 1 beat 1

[tempo]                 # bar where it takes effect = BPM
1   = 120
33  = 128

[cues]                  # label = bar.beat   (or  @seconds for off-grid)
intro     = 1.1         # -> 0.35 s
build     = 9.1         # -> 16.35 s   (see worked tempo-boundary example below)
drop      = 17.1
breakdown = 33.1        # tempo lifts to 128 here
reform    = 49.1
hit_kick  = @92.500     # off-grid sample, raw seconds
end       = 81.1
```

**The pure-seconds counter (OQ#4's other branch) — same show, two extra lines:**

```
# martin.cues   [PURE-SECONDS — simpler; loses musical re-export]
audio = ascent.ogg
[cues]
intro=0.35  build=16.35  drop=32.35  breakdown=64.35  reform=96.35  end=...
```

Pure seconds is fewer concepts and no tempo math; the cost is that a BPM re-export of
the track invalidates every cue, and "land on the downbeat" is eyeballed not exact.
Beats-first inverts that trade. **Pick one in the morning.**

**The one load-bearing formula, worked across a tempo boundary** (since determinism
is the whole point, do the arithmetic once): `bar.beat → seconds = offset + Σ over
tempo segments of (beats in segment) × (60/BPM)`. For `build = 9.1` at constant
120 BPM from bar 1: that is 8 bars × 4 beats = 32 beats elapsed, each 60/120 = 0.5 s,
so `0.35 + 32×0.5 = 16.35 s`. For a cue *after* the 128-BPM change at bar 33: sum the
bars 1–32 at 0.5 s/beat **then** the remaining beats at 60/128 ≈ 0.46875 s/beat —
the integral splits at the boundary, it is not one flat multiply. `.beat` may be
fractional (`17.3` = bar 17, beat 3), so no separate rows-per-beat field is needed.

### 3.3 How parts reference cues

Add an optional cue anchor to `Part`, resolve to absolute start seconds at build, and
have `part_director` find the active part by absolute time (it currently walks parts
summing `seg = morph + hold`, `main.rs:467-476`). New `parse_seq` syntax coexisting
with `@hold,morph,bulge`:

- `@@drop` — this part **starts** at cue `drop`; morph uses its explicit/default
  `@morph`; **holds** until the next anchored part. The common case.
- `@@drop->reform` — start at `drop`, **morph runs until the next cue** `reform`
  (morph = `t(reform) - t(drop)`), landing complete exactly on `reform`.
- Un-anchored parts keep today's relative semantics (laid end-to-end) — a show with
  no cue file behaves **exactly as today**.

A `resolve_cues(parts, &cue_map)` pass computes each part's `start_secs`; clamps
negative `hold` to 0 with a loud `warn!` (a cue landed before the morph could finish
— the one place authoring mistakes surface). `part_director` then selects "last part
whose `start_secs <= clock.t`" and `factor = ((t-start)/morph)`. Everything
downstream (lhs/rhs retarget, ease, bulge gate) is **unchanged** — we only change how
`idx`/`factor` derive from `t`, preserving #3/#4. This is the degenerate-Rocket: the
cue map is a single labeled "track," `part_director` the interpolating client.

### 3.4 Keeping RECORD mode deterministic AND music-synced

Constraint #5 is sacred. In record mode we **invert** the relationship: derive the
audio position *from* the frame index, and mux audio offline. The current line
(`main.rs:568`) is already correct:

```rust
clock.t = i as f32 * rec.dt;   // record: frame index IS the audio position; AV_OFFSET not applied
```

`record_driver` produces silent PNG frames; the existing ffmpeg step muxes the cue
sheet's `audio` starting at video time 0 with the same `offset` baked in. Since frame
`i` shows music-time `i*dt` and audio starts at music-time 0, they are sample-locked
**by construction** — no drift possible. One small change: extend `dur` so `total`
covers the audio length (`dur = max(Σ part durations, audio_length) + tail`). The
recorder never reads the live `MusicClock` (it already bails, `main.rs:552`). This is
strictly better than live sync: the recorded video is the reference AV alignment,
reproducible bit-for-bit. (→ OQ#6 confirms offline-mux vs any live capture.)

### 3.5 Latency & drift (three issues, three fixes)

1. **Output latency (live only):** soundcard buffers ~10–40 ms. Rocket's trick — one
   tunable `AV_OFFSET ≈ +0.03..0.08 s` (`MARTIN_AV_OFFSET`, default ~0.04; positive =
   visuals run slightly ahead to meet the delayed sound). Not applied in record mode.
   (→ OQ#7: per-show vs global env.)
2. **Clock drift (live only):** solved structurally by §3.1 (slave to audio each
   frame). Impl (B) removes the residual entirely if ever needed. Record mode has
   zero drift by construction.
3. **Cue-vs-morph collision (authoring):** caught at `resolve_cues` — clamp + warn,
   turning a silent visual desync into a build-time log line.

**Code touch points (all app-side, no shader change — respects #3/#4):** wire actual
audio playback (new); new `MusicClock` resource; new `src/cues.rs`; `parse_seq`
`#cues:` directive + `@@cue` anchors; `resolve_cues` + `start_secs: Vec<f32>` in
`SeqState`; `advance_seq_clock` swap; `record_driver` `dur` extension + offline mux;
`MARTIN_AV_OFFSET` env. **Deferred:** a live socket editor; multi-track float tracks.

---

## 4. Transition catalog — what ships, what's the gap

The ball-pulse is **one** transition. The registry already exists: the `Transition`
enum + `Transition::parse` + `MARTIN_TRANSITION` + `build_sequence`'s per-part source
match (§0.1). A new transition is **one enum variant + one `morph.rs` fn + one
`build_sequence` arm** — not a new abstraction.

The lever depends on the tier:

- **DATA-ONLY (Group A):** a pure `fn(shape) -> Vec<Gaussian3d>` that builds the
  `lhs` source cloud. Each particle is paired k↔k by `resample_morton`, so `lhs[k]`
  morphs into `rhs[k]`; a data-only transition just decides *where particle k starts
  and at what opacity/colour* — the GPU lerps the rest (positions, opacity,
  rotation-then-normalized, SH). **No fork edit.**
- **SHADER (Group B):** anything needing **per-particle staggered timing** — the
  single global `time` can't express it. Requires the gated WGSL block + one uniform
  (the §5 blueprint, gated behind OQ#12).

Field cheat-sheet (real, from `morph.rs`/`text.rs`):
`g.position_visibility.position`, `g.scale_opacity.scale`/`.opacity`,
`g.spherical_harmonic` (degree-0 via `dc(c)`), `g.rotation = [x,y,z,w].into()`.

### 4.1 Group A — DATA-ONLY: the shipped name map + the genuine gaps

**Already shipped** (in `morph.rs`, wired in `build_sequence:377-388`). Do not
re-implement these:

| Author name | Shipped variant | Shipped fn | Lever |
|---|---|---|---|
| Direct morph | `Morph` | (none — lhs = prev shape) | `bulge=0`, lhs = prev |
| Ball pulse (current default) | `Ball` | `ball_of` | fuzzy shell + `sin(pi*t)` bulge |
| Fade in | `Fade` | `fade_of` | lhs = target, opacity 0 |
| Explode-away-in | `Explode` | `explode_of` | lhs = outward burst, gathers in |
| Implode-in | `Implode` | `implode_of` | lhs = dense speck, expands out |
| Gravity drop | `Drop` | `drop_of` | lhs = target lifted +Y, falls |
| Swirl / vortex (cheap) | `Swirl` | `swirl_of` | lhs = Y-rotated + expanded ring |

So the line `splat:aegg.ply ~fade @2,3.5,0` **already works today.** The map exists
because the old draft proposed rival names (`~slide`, `~gravity`, `~shatter`,
`~vortex`, `~dissolve`, `~direct`) that the parser does **not** accept — those would
be new variants, listed under "gaps" below.

**Genuine new DATA-ONLY gaps (one fn + one enum variant + one arm each):**

- **Dissolve / crossfade** — lhs = previous shape (not a built source), so Morton
  pairing + per-particle opacity lerp does an in-place A→B crossfade. Cheapest gap:
  it is `Morph` with `bulge=0` plus letting the prev shape's opacity ride down — but
  *partial-opacity* fades on a moving cloud are **asserted sort-safe, not proven**;
  only the full-fade case (`fade_of`, opacity-0 lhs) is verified to survive the
  renderer's opacity early-out. Mark "needs a render check." *Effort: trivial-ish.*
- **Directional wipe (soft slide)** — lhs = target offset along an axis by each
  particle's own coordinate along that axis, trailing edge fainter; reads as a slab
  sliding in. New `wipe_of(shape, span)`. *Effort: trivial.* (A *hard* reveal edge
  needs per-particle `t` → SHADER typewriter, §4.2 B1.)
- **Shatter / scatter** — lhs = target with positions randomized in a box **and** the
  `rotation` quaternion randomized; the GPU normalizes the lerped rotation, so
  tumbling reads naturally. This is the one A-tier idea that touches a field
  (`rotation`) none of the *existing* transitions touch — slightly more than trivial.
  New `shatter_of`. *Effort: trivial-to-moderate.*

*Sketch — the wipe gap, in the existing `*_of` style (note: `wipe_of`, not
`wipe_source`):*

```rust
/// WIPE source: soft left→right slide. lhs = target offset by its own x, trailing edge faded.
pub fn wipe_of(shape: &[Gaussian3d], span: f32) -> Vec<Gaussian3d> {
    let (mut lo, mut hi) = (f32::MAX, f32::MIN);
    for g in shape { let x = g.position_visibility.position[0]; lo = lo.min(x); hi = hi.max(x); }
    let inv = 1.0 / (hi - lo).max(1e-6);
    shape.iter().map(|g| {
        let p = g.position_visibility.position;
        let frac = (p[0] - lo) * inv;
        let mut s = *g;
        s.position_visibility = [p[0] - span * frac, p[1], p[2], 1.0].into();
        let sc = g.scale_opacity.scale; let op = g.scale_opacity.opacity;
        s.scale_opacity = [sc[0], sc[1], sc[2], op * (1.0 - frac)].into(); // partial fade: render-check
        s
    }).collect()
}
```

Wiring it = add `Wipe` to the enum + a `Transition::Wipe => Some(wipe_of(&shaped, r))`
arm next to the existing ones (`main.rs:381`). One representative sketch is enough to
decide the shape; the other gaps follow the same pattern and don't need finished
bodies to argue about.

### 4.2 Group B — SHADER (one gated WGSL block + one uniform; gated behind OQ#12)

All need per-particle staggered timing — the global `time` can't express it. Pattern:
a small block in `vs_points` (next to the ball-pulse, gated by a new uniform, off by
default) deriving a per-particle phase from `splat_index`/position, then gating
opacity or displacing position. Determinism is automatic — everything is a function
of `splat_index` + `time`, and `record_driver` sets `time` from frame index.

> **Canonical-design note:** §5 is the **single unifying blueprint** that implements
> all of B1–B5 with **one** uniform group (`transition_mode` + softness + axis).
> The illustrative WGSL fragments below show *intent per effect*; they are **not**
> separate uniforms. **§5 supersedes them** — do not add a `sparkle_mode` /
> `slither_mode` uniform; everything routes through the one `transition_mode` switch.

- **B1 · Typewriter** — hard left→right reveal edge. Reveal particle when `t` passes
  its normalized x. For text, a local-x wipe gives true left-to-right; mode 5's pure
  index order is the fallback. *Effort: moderate.*
- **B2 · Write-with-a-pen** — reveal follows the *pen path*, not a straight sweep.
  **This is the real risk, not a one-liner.** `text.rs` today rasterizes glyph
  *coverage* (it samples filled pixels) — it does **not** walk outline contours. A
  true pen order needs a **new outline-walk path** in `text.rs` (e.g. `ab_glyph`
  `OutlineCurve` segments → cumulative pen distance per emitted gaussian), packed
  into the unused text `z` and read as the phase. That is a different code path from
  the coverage sampler, plus the shader gate. *Effort: hard.* (→ OQ#12.)
- **B3 · Slither text** — letters slither in on a sine wave, staggered by x, amplitude
  dying as each settles. *Effort: moderate.*
- **B4 · Sparkle in / spark out** — random per-particle reveal time keyed by
  `explode_hash3(splat_index)` + a brief over-bright flash (the HDR `Bloom` already
  in the pipeline makes it twinkle). "Spark out" is the reversed reveal — **but see
  the timeline gap in §4.4: there is no dedicated "leaving" phase today.** *Effort:
  moderate (reveal) / harder (true leaving).*
- **B5 · Vortex (true, t-driven)** — rotate `position` about the object axis by
  `angle = (1-t)*turns*f(radius)`, unwinding to zero as it lands. The shipped `Swirl`
  is the cheap data-only approximation; this is the continuous version. *Effort:
  moderate.*
- **B6 · Explode / implode OUT (leaving)** — the engine **already has** the
  ballistic+gravity explode in `gaussian.wgsl` (the `explode_t != 0.0 && !interp_active`
  branch, ~lines 220–236), but it is gated OFF whenever `interp_active`. Using it as a
  transition-*out* means letting the displacement run on the tail **while a morph is
  active** — so explode and morph then contend for the same global `time` scalar.
  That is more than "gating": it is reconciling two consumers of one `time`. **Effort:
  moderate-to-hard, not moderate.** (And it needs the out-transition timeline slot of
  §4.4.)

### 4.3 Classification table

| Transition | Status | Class | Effort | Lever |
|---|---|---|---|---|
| Morph (direct) | **shipped** | DATA | trivial | `bulge=0`, lhs=prev |
| Ball pulse | **shipped** | HYBRID | done | `ball_of` + `cs.bulge` |
| Fade in | **shipped** | DATA | done | `fade_of`, opacity 0 |
| Explode-away-in | **shipped** | DATA | done | `explode_of` |
| Implode-in | **shipped** | DATA | done | `implode_of` |
| Gravity drop | **shipped** | DATA | done | `drop_of` |
| Swirl (cheap) | **shipped** | DATA | done | `swirl_of` |
| Dissolve/crossfade | gap | DATA | trivial* | lhs=prev + opacity ride (*render-check) |
| Directional wipe (soft) | gap | DATA | trivial | `wipe_of`, axis-staggered slide |
| Shatter/scatter | gap | DATA | trivial-mod | `shatter_of`, box + tumble (touches rotation) |
| B1 Typewriter | gap | SHADER | moderate | per-x reveal smoothstep |
| B2 Write-with-pen | gap | SHADER+CPU | **hard** | **new outline-walk in text.rs** + reveal |
| B3 Slither text | gap | SHADER | moderate | staggered local-t + sine |
| B4 Sparkle / spark | gap | SHADER | mod / harder | hashed reveal + flash (+ §4.4 for out) |
| B5 Vortex (true) | gap | SHADER | moderate | (1-t) rotation about axis |
| B6 Explode/implode OUT | gap | SHADER | **mod-hard** | ungate explode during morph tail (+§4.4) |

\* partial-opacity fade on a moving cloud is asserted sort-safe; only full fade is
proven (`fade_of`). Needs a render check before promotion to "done."

### 4.4 The missing timeline concept: out-transitions

Both "spark out" and "explode away as a part leaves" are first-class asks, but the
timeline model has **no leaving phase**: today a part *holds*, then the **next**
part's morph-in begins (`part_director`, `main.rs:467-494`). There is no dedicated
out-transition slot. The shipped `*_of` sources are all *in*-transitions (they build
the `lhs` the morph flies *from*). A true out-transition needs either (a) a
displacement on the *outgoing* `rhs`/tail side, or (b) a synthetic zero-content part
the previous shape explodes into. This is a **timeline-model gap**, separate from the
shader gap — call it out explicitly so the morning conversation can decide whether
out-transitions are worth a new timeline slot (→ OQ#12 covers the shader half; the
slot itself is a fresh open item, added to §9.2 #16).

---

## 5. The one deliberate fork edit — per-particle phase (blueprint sketch only)

> **This is gated behind OQ#12 — the decision to do step 8 at all.** The shader edit
> is martin's *single deliberate fork edit*, the thing to **co-design**, not arrive
> pre-written. So this section is a one-page sketch; the full WGSL helper, the gated
> branch, the 4-spot Rust plumbing, the risk table, and the upstream-PR shape live in
> **`SHADER-BLUEPRINT.md`**, marked "reference for when we decide to do step 8."

**The idea (covers B1–B5 with one uniform group, default-off, ~`bulge`-sized
footprint).** Today `gaussian_uniforms.time` is one blend factor `t∈[0,1]`, identical
for every particle. All of B1–B5 share one structure: **each particle gets its own
scalar `phase ∈ [0,1]` (from its index and/or local position), and the global `time`
sweeps a moving window across that phase axis:**

```
local = saturate((global_t * (1 + softness) - phase) / softness)
```

`local` ramps 0→1 as the front passes the particle's phase; feed it into opacity
(reveal/sparkle), the ball-pulse (staggered disperse), or position (slither/pen
lead-in). **Mode 0 reproduces today bit-for-bit** by never touching `local`.

**Why it's a clean PR (the load-bearing facts, verified):**

- **It mirrors the landed `bulge` feature exactly** — one helper fn + one gated
  branch + a one-line opacity multiply at `output.color` (verified live:
  `opacity * gaussian_uniforms.global_opacity` at `gaussian.wgsl:480-482`; the
  `* tx_reveal` edit is algebraically identical when `tx_reveal == 1.0`).
- **One uniform group plumbed in the same 4 spots** as `bulge`
  (`bindings.wgsl`, `render/mod.rs` struct, `render/mod.rs` construction,
  `settings.rs`). Default-off (`transition_mode: 0`) + append-only ⇒ cannot regress
  existing users.
- **CRITICAL append location.** The **verified** live layout
  (`bindings.wgsl:13-27`) ends:
  `… time_start, time_stop, bulge, num_classes: u32, color_space: u32, min: vec4,
  max: vec4`. So the new fields must be **appended after `max: vec4` (the true
  struct end)** — **NOT** after `bulge`, which would shift `num_classes`,
  `color_space`, `min`, `max` and corrupt every offset (`CloudUniform` mirrors this
  order at `mod.rs:970-983`, constructed at `mod.rs:1031-1048`). This one location is
  the difference between a clean PR
  and silently garbage uniforms. (Note `CloudSettings` is *richer* than the uniform
  — it also has `time_scale`/`num_classes`/`color_space`, `gaussian/settings.rs:71-76`
  (struct 60-81) — so
  "identical footprint as `bulge`" is true of the *uniform*, not the whole settings
  struct.)
- **The GPU sort doesn't see it.** The radix sort is a compute pass over the
  morph-output buffer; `vs_points` runs later, so a vertex-shader position nudge is
  invisible to the sort — exactly like the existing bulge pulse. Opacity-only modes
  are fully sort-safe. **Do NOT move any of this into `interpolate.wgsl`** (that
  buffer *is* what the sort reads). This is the cheapest, most correct insight in the
  whole shader story and it carries over verbatim from the bulge work.

**App side:** set the new fields next to `cs.bulge =` in `part_director`
(`main.rs:494`), keyed off the part's transition. `time` is already the eased,
frame-indexed blend factor, so the sweep is automatically deterministic in record
mode (#5 holds: phase uses only `splat_index` + position + uniforms — never
wall-clock, never RNG state). Bulge and the transition are orthogonal and composable.

Everything beyond this paragraph — the `transition_phase` helper, the full gated
branch, the four Rust hunks, the std140/`_transition_pad` alignment note, the
`switch`-on-uniform RADV fallback, and mode-5-vs-resample caveat — is in
`SHADER-BLUEPRINT.md`. Pull it in only once OQ#12 says "yes, do step 8."

---

## 6. Scene composition

Today a part is one homogeneous cloud (one entity, one `N`). The goal: a part as a
*scene* — several heterogeneous elements (splat objects, splat-text, a logo image,
later meshes), each with its own transform, still morphing between scenes. The
central tension: keep the "one merged cloud" invariant (Option A, pay at build) or
relax to multiple entities (Option B, pay at runtime)? The honest leaning is a
**hybrid** — **but be blunt: the hybrid relaxes two of the six hard constraints
(#2 and #3).** (→ OQ#8.)

### 6.1 Option A — compose into ONE merged cloud per part (the constraint-honoring default)

The minimal, constraint-honoring evolution. `part_gaussians` already does a baby
version (load several splats, offset each, concat, `main.rs:272-294`); generalize
"offset" → "full affine transform + per-element normalize," and add element kinds:

```
for each element e in part.elements:
    raw_e = normalize_e(load_element(e))          # per-element, BEFORE transform
    raw_e = apply_transform(raw_e, e.transform)   # S·R·T on positions+gaussian scale+SH rotation
merged = concat(all raw_e)
shaped = resample_morton(merged, N)               # unchanged: one cloud, one N
```

The merged cloud flows through the **entire existing engine unchanged** — one shape,
`ball_of`, one entity, one `time`, the bulge, record determinism — because the output
is still a flat `Vec<Gaussian3d>`. **Stays inside all six constraints.**

**Trade-offs.** Morphing is *excellent and free* (a logo can literally morph into text
into a splat — they're one buffer; the whole reason the engine exists). One radix sort
= correct depth order, no inter-element transparency artifacts. Best perf. **Cost:**
per-element normalize must be *opt-out, not global* — today `MARTIN_NORMALIZE`
normalizes the whole part (`build_sequence:323-332`); normalizing the union after
merging destroys the relative sizing transforms set, so normalize moves *inside* the
element loop. Count budget is shared (a 5-pixel logo and a 1M-splat object both
resample to `N`). **Limitation:** cannot host a non-gaussian `Mesh3d` — that forces
Option B (or point-sampling, §6.5).

### 6.2 Option B — multiple entities (relaxes constraints #2 and #3 — say it plainly)

Spawn N `GaussianInterpolate` entities per part, each with its own
`lhs/rhs/CloudSettings/Transform`. **This directly relaxes constraint #3 (exactly ONE
entity) and constraint #2 (one shared `N`).** Per the doc's own rule — constraints are
ground truth "unless it explicitly proposes a fork edit and says so" — *this is that
explicit relaxation, flagged here.* It is not a shader/vendor fork; it's an app-side
structural change.

**Trade-offs:** per-element *independent* morph/hold becomes possible (new expressive
power), but cross-element morph (logo→text) becomes *impossible within a track*
(separate buffers) — you cross-fade opacity instead, losing the signature effect. N
sorts, no global depth order (fine for spatially separated elements; artifacts for
overlapping translucent ones — don't try to solve OIT). Per-element `N` is the *safe*
form of relaxing #2 because the rule is per-buffer: a logo at 8k, an object at 400k —
more efficient than forcing the logo up to a shared 200k. Constraint #4 still bites:
`time` is per-entity, so staggering is *between* elements, not *within* one.

### 6.3 The hybrid (leaning): merge-groups within a part (→ OQ#8)

**Option A *inside* a track, Option B *across* tracks.** A part has one or more
*tracks*; each track is one entity. Elements sharing a track are merged and morph
together; elements in different tracks are independent entities composed by transform
/ depth.

- Default: all `splat|text|image` elements with no explicit `track` go in **track 0**
  → merged → one entity → morphs across parts exactly like today. **100% backward
  compatible:** a single-element part is identical to current behaviour.
- A `mesh` element is forced to its own mesh track (native `Mesh3d`, never merged).
- An element tagged `track:logo` gets its own gaussian entity that can persist/hold
  independently (a held deFEEST logo across part changes).

It degrades gracefully: author everything in track 0 and you have exactly today's
engine. But see §6.8 — the multi-track machinery is the **biggest** structural change
in this whole doc, not a small one.

### 6.4 IMAGE / LOGO → gaussians (`build_image_gaussians`) (→ OQ#10)

The deFEEST logo (`assets/defeest-logo.png` — verified present, **300×110 RGBA, 5,361
bytes**) becomes flat z=0 colored gaussians, then the engine treats it identically to
text (it can ball-assemble, hold, morph into the next scene). This is the **primary**
path for the logo; OQ#10 keeps the baked-`.ply`-from-`.dae` alternative open (sharper
edges, offline step). A near-clone of `build_text_gaussians`, in a new `src/image.rs`:

```rust
//! Splat-image: sample opaque pixels of a PNG into flat (z=0) colored gaussians, so a logo
//! is just another morph source. Built Y-DOWN so cloud_base_rotation flips it upright.
use bevy_gaussian_splatting::{Gaussian3d, SphericalHarmonicCoefficients};

fn dc(c: f32) -> f32 { (c - 0.5) / 0.282_094_79 }   // mirror text::dc, degree-0 SH encode

pub fn build_image_gaussians(png: &[u8], world_width: f32, stride: usize,
                             splat: f32, alpha_thresh: f32) -> Vec<Gaussian3d> {
    let img = image::load_from_memory(png).expect("png").to_rgba8();
    let (w, h) = (img.width(), img.height());
    let scale = world_width / w.max(1) as f32;
    let (cx, cy) = (w as f32 * 0.5, h as f32 * 0.5);
    let mut out = Vec::new();
    let mut i: u32 = 0;
    for yy in (0..h).step_by(stride) {
        for xx in (0..w).step_by(stride) {
            let px = img.get_pixel(xx, yy);
            let a = px[3] as f32 / 255.0;
            if a < alpha_thresh { continue; }                    // opaque pixels only
            let (r, g, b) = (px[0] as f32/255.0, px[1] as f32/255.0, px[2] as f32/255.0);
            let mut sh = SphericalHarmonicCoefficients::default();
            sh.set(0, dc(r)); sh.set(1, dc(g)); sh.set(2, dc(b));
            let j = |k: u32| ((k.wrapping_mul(2_654_435_761) >> 8) & 0xff) as f32 / 255.0 - 0.5;
            let gx = (xx as f32 + j(i) * stride as f32 - cx) * scale;
            let gy = (yy as f32 + j(i ^ 0x9e37) * stride as f32 - cy) * scale;  // Y-DOWN
            i = i.wrapping_add(1);
            out.push(Gaussian3d {
                position_visibility: [gx, gy, 0.0, 1.0].into(),
                spherical_harmonic: sh,
                rotation: [0.0, 0.0, 0.0, 1.0].into(),
                scale_opacity: [splat, splat, splat, a].into(),
            });
        }
    }
    out
}
```

Slot-in notes: deterministic jitter + Y-down copied verbatim from `text.rs` (record
determinism #5); color path identical to text (consider a sub-1 gain so bright logos
don't bloom out, mirroring `TEXT_RGB`'s 0.8 — → OQ#11); load the PNG bytes with
`std::fs::read(root/name)` in the build phase, resolving `image:defeest-logo.png`
against the same root as `splat:`. **Dep cost:** `image` is **not** a direct dependency
today (`Cargo.toml` has only `bevy`, `bevy_gaussian_splatting`, `ab_glyph`). Bevy pulls
`image` transitively, but relying on that is fragile across Bevy bumps — **add `image`
explicitly to `Cargo.toml`** rather than asserting it's free.

### 6.5 Meshes — a separate, parallel track (→ OQ#15)

A `Mesh3d` has connectivity, is lit, rasterizes opaque triangles — it **cannot live in
the morph buffer** (the interpolate compute shader blends gaussian fields only). So
meshes are Option B by necessity:

- **Native (recommended):** a standard `Mesh3d` + `MeshMaterial3d` with its own
  `Transform`, composed into the same camera/scene. The orbit camera already frames
  it.
- **Transitions for meshes** are not gaussian morphs: scale-in/out, material-alpha
  fade, transform tween — keyed off the same frame-indexed `clock.t`. (Determinism
  caveat: see §6.7.)
- **Depth:** opaque mesh writes depth; the gaussian sort is independent — fine when
  the mesh is behind/beside the cloud, imperfect if a translucent cloud must show both
  through *and* in front of a mesh. Document; don't solve OIT.
- **Escape hatch (later):** point-sample the mesh surface into N gaussians so it morphs
  in track 0 — losing lighting/sharpness. The repo has `bitterbal.obj` and **two
  logo meshes, `defeest.dae` and `deFEEST.dae` (identical, 465,091 bytes each)** —
  pick one canonical name in the morning (→ OQ#10/#15).

### 6.6 Per-element schema — minimal spine first, OQ-gated extras

The old draft committed 6 struct fields up front; that pre-empts OQ#8/#9. Present the
**minimal spine** (kind / transform / track), and list the rest as
*add-if-the-OQ-goes-that-way*:

```rust
enum ElementKind {
    Splat { src: String },   Text { string: String },
    Image { src: String },   Mesh { src: String },
}
struct ElementTransform { position: Vec3, rotation: Quat, scale: Vec3 }
struct Element {                 // --- minimal spine ---
    kind: ElementKind,
    transform: ElementTransform, // S·R·T on positions; R rotates SH; S scales gaussian scale
    track: TrackId,              // 0 = merged morph track (default); named = own entity (OQ#8)
    // --- fields we ADD only if the OQ goes that way ---
    // normalize: Option<f32>,   // per-element extent before transform (OQ#9)
    // count_weight: f32,        // share of N (OQ#9 — undecided whether it's worth it)
    // as_gaussians: bool,       // mesh only: point-sample into track 0 (OQ#15)
}
enum PartContent {
    Text(String),                // KEPT — desugars to one Text element, track 0
    Splats(Vec<(String, Vec3)>), // KEPT — desugars to splat elements, track 0
    Compose(Vec<Element>),       // NEW
}
```

**Grammar** (extends `parse_seq`, stays line/`;`-based): a `compose:` part whose body
is `|`-separated elements; each element is `kind:src` plus optional `@@`-prefixed
transform tokens (chosen not to collide with the part-level `@hold,morph,bulge`):

```
compose: splat:doggo.ply        @@pos=0,0,0
       | image:defeest-logo.png @@pos=-1.2,0.6,0 scale=0.8 track=logo
       | text:deFEEST           @@pos=0,-1.1,0 scale=0.5
       @ 2.0, 3.0, 0.9          # part-level timing unchanged
```

Unknown `@@` tokens are ignored (same forgiving `filter_map` spirit as the current
parser, `main.rs:240`).

### 6.7 Determinism of multi-entity / mesh tracks (not yet shown — flag it)

§6.2/§6.5 *assert* that per-track `time` and mesh material-alpha tweens "stay a pure
function of frame-indexed `clock.t`." That has to be **demonstrated**, not asserted,
because `record_driver` today drives exactly **one** clock and **one** entity
(`state.entity: Option<Entity>`, `main.rs:204`; retarget at `main.rs:480-489`). For N
deterministic entities + mesh tracks in record mode you must: (a) generalize
`state.entity` to a `Vec<TrackState>`; (b) have a single `part_director` set every
track's `time`/`Transform`/material-alpha from the one frame-indexed `clock.t`;
(c) ensure any new system replicates the `controls`/`advance_seq_clock` record-mode
bail (`main.rs:99-101, 504-506`). Until that path exists, multi-track determinism is a
plan, not a fact — say so.

### 6.8 What changes in code — and it is NOT "surgical"

`src/image.rs` (new) + `image` in `Cargo.toml`; `apply_transform` in `morph.rs`
(positions, gaussian scale, SH rotation; `normalize_to` exists); `main.rs`:
`Compose(Vec<Element>)` added (Text/Splats desugar so existing loaders iterate
elements), `build_sequence` groups by track. **The big one:** `part_director` is
hard-wired to a *single* `state.entity` (`main.rs:460-461, 480-489`); track-based
composition requires generalizing it (and `state.entity`, and the record path, §6.7)
to a `Vec<TrackState>`. **This is the largest structural change proposed in this whole
document — label it HARD, not surgical.** Option A (one merged cloud) avoids all of it
and stays inside the constraints, which is exactly why it's the safe v1 and the hybrid
is the ceiling (→ OQ#8). **No WGSL/vendor edit is required** for either A or the
track-based B — the fork stays as-is unless you later want staggered time *within* a
track (then it's §5).

---

## 7. Branding / credits / greets

martin is a deFEEST production, co-written by **annejan** and **Kloot** (the AI's scene
handle). Demoscene credit beats, mapped onto the part model.

### 7.1 Identity rules (must hold in published output)

- **On-screen name:** `martin` (or `MARTIN`) — what the audience and the executable are
  called. `deFEEST presents` may sting before it.
- **Kloot is the AI. Credit it as `Kloot`, full stop.** Never `Claude & Kloot`, never
  `Claude (Kloot)` — that's one entity twice. `annejan & Kloot` is the human+AI pair.
- **Do NOT print "Evoke" anywhere** in published output (titles, credits, scroller,
  logo beats). "Evoke" survives only as part of the repo slug
  `github.com/annejan/evoke-martin`, which may appear as a bare source link.
- **deFEEST** spelling is stylised (lower `de`, upper `FEEST`); keep it consistent.
  **Cinder (deFEEST)** is music; keep the group tag on first mention.

### 7.2 Credits become parts

The four traditional credit beats — **title**, **code/music/gfx**, **greetings**,
**end-scroller** — are just more parts. Two carriers: splat-text (`text:...`, the
workhorse — multi-line via `\n` → real line layout in `text.rs`) and the logo as
gaussians (§6.4, or a baked `defeest.ply` from `defeest.dae`). A true scrolling marquee
isn't possible (per-particle motion needs §5), so the end-scroller is a few held text
parts.

**Parser caveat:** the `@hold,morph,bulge` tail splits on commas (`main.rs:240`), and
the head is split off by `@` first — so the comma hazard bites only when a `text:` line
*also* carries an `@` tail. Safest rule: **keep commas out of `text:` payloads** — use
`·`, `/`, or `\n`. The credit line stacked:

```
CODE  annejan & Kloot
MUSIC  Cinder (deFEEST)
A deFEEST PRODUCTION
```

### 7.3 Where the logo lives & its transition (honest)

Bookend the show: **intro** `deFEEST presents` → logo → `martin`; **outro** the logo
returns as the final held mark (classic group intro+outro bumper). martin's transition
primitives are now several (§4.1 — `Ball`/`Fade`/etc. all ship), but for a logo:

- **"assemble + glow" — fully achievable now.** Use a **low bulge** (logos read badly
  blown apart); the glow is free (HDR `Bloom` on the bright logo as it settles). Intro
  logo `@2.5,3.0,0.4`; `bulge 0` (or `~fade`) gives the cleanest "pixels snap into the
  mark" assemble; `0.3–0.5` a gentle breathing halo. **Keep logos ≤ 0.5 bulge.**
- **"pen-write" — not possible without a shader change.** No per-particle phase today
  (#4), and — the part the old draft underplayed — a pen path needs a **new
  outline-walk in `text.rs`**, not an extension of the coverage rasterizer (§4.2 B2).
  It's a lovely future fork edit (the §5 blueprint, mode 4) but out of scope for
  today's engine. For now, assemble+glow is the right in-budget choice.

### 7.4 Copy-paste branded show (`show.seq`)

```text
# martin — a deFEEST production. code: annejan & Kloot / music: Cinder (deFEEST)
# ---- INTRO: group sting -> logo -> title ----
text:deFEEST\npresents          @1.5,2.0,0.6
splat:defeest.ply               @2.5,3.0,0.4    # logo: assemble + glow, low bulge
text:martin                     @2.5,3.0,0.5    # the title

# ---- BODY (example) ----
splat:martin.ply+martin-peace.ply  @2.0,3.0,0.6
splat:doggo.ply                    @2.0,3.5,0.9 ~ball   # ~ball already works

# ---- GREETS ----
text:GREETINGS                  @1.5,2.5,0.9
text:to everyone\nstill rendering\non the metal   @2.0,2.5,0.8

# ---- CREDITS (stacked, comma-free) ----
text:CODE\nannejan & Kloot      @2.5,3.0,0.5
text:MUSIC\nCinder (deFEEST)    @2.5,3.0,0.5
text:a deFEEST\nproduction      @2.5,3.0,0.4

# ---- OUTRO: logo signs off (calm assemble, no explosion) ----
splat:defeest.ply               @4.0,3.0,0.0
```

Text-only fallback (no logo `.ply`/PNG part yet): swap the two `splat:defeest.ply`
lines for `text:deFEEST`. End-scroller beats use held text and the repo link as a bare
slug (`github.com/annejan/evoke-martin`).

### 7.5 Style guardrails

Logos `bulge ≤ 0.5`; credits/greets `0.5–0.9` (celebratory energy); hold logos longer
(2.5–4.0) than transient greets (1.5); title and outro logo end on `bulge 0` (calm, no
fireworks on the most identity-critical beats); keep `Kloot` exactly as `Kloot`; never
surface "Evoke" or "Claude" in any on-screen string.

### 7.6 One small engine ask (optional, for the logo to be first-class)

The only capability gap is **PNG→gaussians** — `build_image_gaussians` (§6.4) +
`PartContent::Image` + an `image:` prefix in `parse_seq` (+ the `image` dep).
Then the logo is a true part with no offline `.ply` bake. "Pen-write" is the separate,
larger future fork edit (§5, mode 4, gated by OQ#12).

---

## 8. The single-binary bundle tie-in

G4 ties §2.4 (script embed), §6.4 (logo embed), and §7 (branding) together. The
demoscene north star here is the **64k/4k intro tradition — Farbrausch's `fr-08:
.the .product`, `.kkrieger`/Werkkzeug** — *store the recipe, not the asset.* It applies
**partially**: text splats and the `.seq` are genuine recipes
(`build_text_gaussians` from font+string is the pattern done right; so is the `.seq`),
but the scanned multi-GB `.ply` are **not** a cheap recipe and won't be procedurally
regenerated.

So "single binary" honestly means: **binary + script + font (+ small logo) embedded;
`.ply` loaded from an asset folder beside it.** Concretely:

- Embed the default show (`include_str!`) and font (already done), optionally the logo
  PNG (`include_bytes!`, **verified ~5 KB / 5,361 bytes**).
- Resolution order (§2.4) makes `./martin` with no env play the embedded default;
  default the asset root to `assets/` on fallback.
- A loader-screen state covers the wait while `.ply` load (the slow part). Reuse the
  existing build-phase wait (`build_sequence` already gates on splats being loaded,
  `main.rs:309`) — show a "deFEEST" splash until `state.built` (→ OQ#14).
- For a real packer, compress the binary with a standard tool — the demoscene
  canonical here is **kkrunchy** (64k) / **Crinkler** (4k) for executable packing;
  `upx` is the plain alternative. Accept `.ply` stays external on disk (still
  uncompressed PLY per #6 — SuperSplat-compressed is rejected by the loader). **Do
  not** promise a zero-file binary referencing `bicycle.ply` (1.52 GB).

Open fork: one-binary-per-show (each its own embedded default) vs one binary + loose
shows (→ OQ#13).

---

## 9. Incremental plan + open questions

### 9.1 Open questions first (the genuine forks for the morning)

*(Questions before the roadmap on purpose — several roadmap steps **are** these
questions. The 5 highest-stakes are also in the §0 box up top.)*

1. **Grammar now:** extend the inline DSL (Option A) only, or jump to RON (Option B)?
   Lean A-now-RON-later, but if composition (§6) lands soon RON earns its deps earlier.
   *(gates roadmap step 9)*
2. **`~transition` syntax:** keep bare `~name`, allow `~name(args)`, or move to a
   `kind=` kv field / RON `transition:`? (The token *exists*; this is the spelling.)
3. **`cue/goto`:** parse-and-ignore now, or skip entirely until a looping director
   exists?
4. **Cue file format:** bars/beats + tempo-map (§3.2 leaning) or pure seconds (§3.2
   counter)? **And** 4/4-only for v1, or general meters? *(gates step 7)*
5. **MusicClock impl:** ship (A) wall-clock-anchored only, or build (B) sample-accurate
   now (+ a `rodio` dep)? Lean (A) behind the seam. *(gates step 6)*
6. **Audio muxing:** confirm offline-ffmpeg-in-post (§3.4) vs any live capture (the
   latter would break determinism).
7. **`AV_OFFSET`:** default value, and per-show vs global env.
8. **Composition backend:** hybrid (A in-track, B across-tracks) as default — accepting
   it **relaxes constraints #2 and #3** (§6.2) — or stay strictly one merged cloud
   until meshes force B? *(gates step 10)*
9. **`count_weight` / per-track `N`:** worth the complexity, or accept the shared budget
   for v1?
10. **Logo source:** raster PNG → `build_image_gaussians` (leaning; ships now, soft
    edges) vs a baked `.ply` from a `.dae` (sharper, offline). *And* which of the two
    duplicate `defeest.dae`/`deFEEST.dae` is canonical? *(gates step 4)*
11. **Logo bloom gain:** add a sub-1 tint to logos (like `TEXT_RGB`), or let them bloom
    hot?
12. **The shader fork (§5 / step 8):** do the one deliberate per-particle-phase edit
    now (unlocks B1–B5), or stay data-only? *(gates step 8)*
13. **Bundling unit:** one-binary-per-show (embedded default each) vs one binary + loose
    `assets/shows/`? *(gates step 0's later evolution)*
14. **Loader screen:** the deFEEST splash, or a minimal progress indicator?
15. **Mesh medium:** keep meshes a non-morphing parallel track, or invest in
    point-sample-to-gaussians so they morph?
16. **Out-transition timeline slot (§4.4):** does the timeline grow a dedicated
    "leaving" phase (for spark-out / explode-away), or do we live without true
    out-transitions? *(new — surfaced by the catalog work)*

### 9.2 Incremental roadmap (small, individually shippable, low-regret)

Each step preserves the `MARTIN_*` shorthands and existing `MARTIN_SEQ` grammar, with a
**golden-frame pixel-identical diff** as the per-step gate (record a known show
before/after; bytes must match for any "no behaviour change" step). Steps are
annotated with their gating OQ where one applies.

0. **Embedded default show + resolution order + default asset root** (§2.4) — makes
   `./martin` self-contained. Highest leverage, syntax-independent. *(→ OQ#13 for the
   bundling-unit evolution.)*
1. **`key=value` timing** alongside positional in `parse_seq` (§2.1) — kills the
   footgun + default drift.
2. **More DATA-ONLY transitions** — the registry, `~name` parser, and `*_of` functions
   **already ship** (§0.1, §4.1). New work is only the *gaps*: `wipe_of`,
   `shatter_of`, dissolve, each as one enum variant + one `morph.rs` fn + one
   `build_sequence` arm. *(→ OQ#2 for the token spelling.)*
3. **`build_image_gaussians` + `image:` prefix + `PartContent::Image` + `image` dep**
   (§6.4) — the deFEEST logo as a first-class part. *(→ OQ#10, OQ#11.)*
4. **Branded default show** using the logo + credit parts (§7) — the show signs itself.
5. **Audio playback wired + `MusicClock` seam + audio-master live clock** (§3.1) —
   replace delta accumulation; record mode untouched. *(→ OQ#5.)*
6. **`src/cues.rs` + `#cues:` + `@@cue` anchors + `resolve_cues`** (§3.2–3.4) —
   music-timed parts; offline audio mux in the existing ffmpeg step. *(→ OQ#4, OQ#6.)*
7. **The §5 shader blueprint** (one gated branch + uniform-in-4-spots, appended after
   `max`) — unlocks B1–B5; isolated commit for the upstream PR. *(→ OQ#12.)*
8. **RON as a second input path** (§2.2) — when parts gain per-part camera / full Vec3
   offsets / music markers. *(→ OQ#1.)*
9. **`compose:` / element schema / track-based composition** (§6) — multi-element
   scenes; per-track entities for held logos and meshes. **The HARD structural change
   (§6.8): generalize the single `state.entity`/`part_director` to `Vec<TrackState>`.**
   *(→ OQ#8, OQ#9.)*
10. **Native mesh track** (§6.5) — `Mesh3d` parallel track, transform/alpha transitions.
    *(→ OQ#15.)*

Steps 0–4 are app-only and shippable now (and 2–4 are *small* given §0.1); 5–6 add the
music seam; 7 is the one deliberate fork edit; 8–10 are the structural growth that
earns RON's deps and (in 9) explicitly relaxes constraints #2/#3.

---

*Co-written by annejan & Kloot, deFEEST. Made on AMD · Vulkan · Bevy.*
