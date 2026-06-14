<!--
SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
SPDX-License-Identifier: MIT
-->

# martin — the domain

> **Status: a design doc, not a spec.** It *defines the domain*, *names the pieces*, and proposes a
> heavy vocabulary restructure measured against established prior art. The naming shortlists in §8 are
> **UNDECIDED** — for the author to pick. The restructure in §9 is a **roadmap**; no code has changed
> on account of this doc. Today's working vocabulary still lives in `USAGE.md`.

## 1. Purpose & scope

martin started as "fly a camera around a Gaussian splat." It is now an **engine with two DSLs bridged
by music, plus a storyboard practice** — and nobody ever defined that domain or named its pieces, so
the vocabulary drifted and borrows film terms ad-hoc. Concrete rot this doc fixes:

- **"transition"** means both the *arrival effect* (`~morph`) and the *GPU blend between two shapes*.
- **"scene"** means both a SHOWBOOK *narrative beat* and `[compose]` *placement*.
- **"cue"** and **"anchor"** are used interchangeably.
- **`morph_count`** is really the *splat budget*, nothing to do with the morph transition.
- The **source cloud** a part flies in from, the **`cluster` "serving"**, and the camera **"regime"**
  have no formal noun at all.

The crafts martin is quietly reinventing already solved this vocabulary problem decades ago:

- **Film grammar** — the hierarchy *shot → scene → sequence*; the transition taxonomy
  *cut / dissolve / fade / wipe*; camera-move verbs *pan / tilt / dolly / truck / zoom*.
- **The animation dope sheet (X-sheet)** — a grid of *rows = time* × *columns = channels*
  (action / dialogue / camera / fx), with held-frame and inbetween notation.
- **GNU Rocket** — the demoscene's standard music↔visual **sync-tracker**: named *tracks* of
  keyframed values over *rows*, rows-per-second derived from BPM, step/linear/smooth interpolation.
  martin's `@@anchor` + `[camera]` track is a *narrow special case* of exactly this.
- **Ad / short-form narrative** — *hook → problem → solution → call-to-action*; the arc
  *exposition → rising action → climax → falling action → resolution*.

This doc steals from all four.

## 2. The domain in one picture

martin is a **music-synchronised, Gaussian-splat, storyboard-driven realtime sequencer**. A
*Production* pairs a musical *Score* (the master clock) with a visual *Show*, designed first on paper
as a *Showbook*. The Score and the Show are two separate DSLs; the bridge between them is the **anchor**
— a symbolic musical position (`@@drop`) that resolves to a **cue** (seconds on the show clock).

```
PRODUCTION   productions/<name>/        the deliverable + its sources
 │
 ├─ SCORE     score.txt                 the MUSIC model — the master clock (bpm · sections · lanes)
 │
 ├─ SHOW      <name>.show               the VISUAL composition
 │   ├─ [settings]                      show-wide knobs (→ MARTIN_*)
 │   ├─ REEL   [reel]  (was [seq])      the single-subject morph timeline
 │   │   └─ SHOT  (was Part)            one held subject + entrance / hold / morph / exit
 │   ├─ STAGE  [stage] (was [compose])  many objects placed at once
 │   │   └─ PROP  (was Composed)        one placed, moving object
 │   └─ CAMERA [camera]                 the keyframed camera track
 │       └─ KEY   (was Waypoint)        one camera keyframe
 │
 └─ SHOWBOOK  SHOWBOOK.md               the STORYBOARD (designed before rendering)
     ├─ ARC                             the ordered narrative beats
     └─ SCENE                           a narrative unit spanning 1+ musical SECTIONS
```

The **anchor/cue bridge** ties the right column to the left: any Shot or camera Key can start at
`@@drop`, `@@bar:14`, `@@beat:4`, `@@start`, or a raw second.

## 3. Canonical vocabulary (the glossary)

Every overloaded or unnamed term gets one home. *Aliased* means the old spelling keeps parsing (martin
already aliases tokens everywhere, so this is cheap).

| Current term | Canonical | Lineage | Change |
|---|---|---|---|
| `martin` (engine) | *(§8a — undecided)* | demoscene one-word names | codename → product name |
| the `.show` file | **Show** | NLE "composition" | noun formalised; extension stays `.show` |
| the `.show` language | *(§8b — undecided)* | — | gets a name |
| `[seq]` | **`[reel]`** | film "reel" | token rename, `[seq]` aliased |
| `Part` (struct) | **Shot** | **film: shot** | the headline rename |
| `[compose]` | **`[stage]`** | theatre / NLE | promote the existing `stage` alias to canonical |
| `Composed` (struct) | **Prop** | stagecraft | struct rename |
| `Waypoint` (struct) | **Key** (camera keyframe) | NLE / Rocket | struct rename |
| `~transition` (the modifier) | **entrance** | dope-sheet "action in"; lifecycle | the `~` slot = *entrance* |
| "transition" (the GPU blend) | **morph** / **cross-morph** | After Effects "blend" | the overloaded word is split off the modifier |
| `out:` (departure) | **`exit:`** | lifecycle "entrance/exit" | token rename, `out:` aliased |
| `^deform` | **deform** ("held motion") | dope-sheet "held action" | kept; documented as held motion |
| `@@anchor` (the spelling) | **anchor** | Rocket row-address | kept; formalised as `AnchorKind` |
| the resolved seconds | **cue** | film "cue" | new precise noun (stop using "cue" for the marker) |
| `morph_count` / `count` | **budget** (splat budget) | — | field/token rename, `morph_count` aliased |
| the source ball/cloud | **origin** | particle "emitter/origin" | names the unnamed |
| `cluster:N` / "serving" | **`flock:N`** | swarm / flock | names the serving, `cluster:` aliased |
| `bg:` | **`backdrop:`** | stagecraft / film | token rename, `bg:` aliased |
| `raster:` | **`raster:`** (render mode) | GPU | kept (it *is* the GPU term) |
| camera "regime" (prose) | **`CameraMove`** enum | **film camera-move verbs** | becomes first-class (§5) |
| SHOWBOOK "scène" | **Scene** (narrative) | **film: scene** | disambiguated from stage placement |
| EDM section name | **Section** (time-grid) | arrangement | kept; Scene *overlays* it (§7) |

**The two load-bearing renames:** `Part → Shot` and SHOWBOOK `scène → Scene`. Together they kill both
worst overloads — a *Shot* is one engine subject, a *Scene* is a narrative span.

## 4. The music model — Score

The **Score** (`score.txt`, tracker-DSL) is the **master timeline**; everything else is timed against
it. It is unchanged by this doc — only named consistently.

- **Score** — `bpm`, key. Owns the clock: `beat = 60/bpm s`, `bar = 4 beats`, 16 steps/bar.
- **Section** — `section <name> <bars> [phases] [fill]`. The **time-grid**, named by EDM arrangement
  (`intro · build · drop · breakdown · climax · outro`). Each Section owns its own chords / patterns /
  dynamics. **Sections are musical truth, not narrative** (see §7).
- **Phase** (`p0..pN`) — drum-energy layers within a Section; melodic lanes ignore phases and loop
  `p0` continuously. **Fill** — the optional trailing flourish bar.
- **Lane** — a sequenced channel. *Drum lanes*: `kick / snare / hat / stab`. *Note lanes*:
  `lead / arp / bass` (16-step grids that loop). *Chords*: a per-bar cycle.
- **Dynamics** — `gain / sub / mids` automation curves, with `a>b` linear **ramps** per Section.
- **FX layer** — demoscene accents (`wall / house / donk / shimmer / riser / jet / impact / bang`),
  default-by-section or overridden via `<section>.fx:`.
- **`set` knobs** — arbitrary synth parameters (`set key=value`, `<section>.set` to override).

Standard DAW/tracker terms (bpm, bar, beat, section, lane, chord, fill) are kept verbatim; the
martin-idiosyncratic ones (phase-as-drum-layer, ramp, the fx accent names) are documented as such.

## 5. The visual model — Show

A **Show** (`.show` file) is sugar that expands to `MARTIN_*` env vars (`src/show.rs`). Three bodies:

### Reel (`[reel]`, was `[seq]`) — the morph timeline
A **Reel** is an ordered list of **Shots**. Each Shot is *one subject that holds the frame*, with a
full lifecycle:

- **content** — `text: / wall: / image: / svg: / mesh: / splat: / glb: / model: / shader:`.
- **timing** — `@hold,morph,bulge` (seconds held · seconds to assemble · ball-pulse explosiveness).
- **entrance** (`~`) — how it arrives: `morph, swarm, ball, fade, explode, implode, drop, rain, funnel,
  shatter, condense, swirl, extrude, helix, fold, zoom` (data) + `typewriter, wipe, sparkle, slither,
  vortex, outline, pen-write` (per-particle shader). It flies in from its **origin** (the source cloud).
- **deform** (`^`) — held motion while on screen: `wave, cloth, ripple, twist, wind, turbulence, pulse,
  jitter, spiral`.
- **exit** (`out:`, → `exit:`) — how it leaves: `wash, disperse, evaporate, sink, explode`.
- **modifiers** — `rot:x,y,z`, `flock:N` (was `cluster:`), `backdrop:NAME` (was `bg:`), `raster:MODE`,
  `@@anchor`.

The **morph** itself (the per-Gaussian GPU blend from one Shot's shape to the next) is a *distinct
concept* from the `~morph` entrance: every Shot cross-morphs into the next regardless of its entrance.
This doc reserves "morph" for the blend and "entrance" for the `~` slot.

#### Pairing — why a morph either *flows* or *balls* (`src/morph.rs`)

A cross-morph is a straight per-particle lerp: splat *k* of shape A → splat *k* of shape B. So the
**pairing** (which B-splat each A-splat is assigned) decides everything. Two strategies:

- **Rank (default)** — `resample_morton`: both shapes are Morton (Z-order) sorted and paired by rank.
  Cheap, and *gorgeous between similar shapes* — a truck and a train occupy the same rough volume, so
  rank ≈ nearest and the lerp slides locally. But between **dissimilar** shapes (city → city) rank
  pairs spatially-distant splats; their lerp **midpoints pile up at the centroid** → the whole cloud
  contracts to a **ball** and re-expands. Geometric, not a bug.
- **Nearest-match** (`MARTIN_PAIR=match`, `match_reorder`) — reorder B so each A-splat pairs with a
  *nearby, similar-colour* B-splat: a greedy bijective match over a voxel grid, cost = `pos² + w·colour²`
  (`MARTIN_PAIR_COLOR`). Every move is short and colour-coherent (grass→grass, tower→tower) → a straight
  ghostly morph, no centre-collapse. The ring search is **radius-capped with a fallback pool** so it
  stays O(n) even as the grid depletes (otherwise the bijection tail is O(n·res³) — minutes at 1M+).

**The other ball source — the beat pulse.** `director.rs` adds `cs.bulge += beat.kick·0.3` *during any
morph* (a deliberate punch on the drop). With `s.bulge = 0` in the show this is the *only* bulge, and
it's strongest at the `@@climax` cut — a real ball even under good pairing. `MARTIN_PAIR=match`
suppresses it, because the whole point of `match` is a straight slide. So "no ball" = good pairing **and**
no beat-pulse; `bulge` (the 3rd timing number) is the explicit, intentional version for when you *want*
the disperse-to-ball.

### Stage (`[stage]`, was `[compose]`) — many at once
A **Stage** holds **Props** — objects placed and animated *simultaneously* (vs the Reel's one-at-a-time
morph). Each Prop: `@x,y,z *scale rot/spin/sway/bob/drift in/out <anchor>` plus an optional `~entrance`
and `^deform`. Lineage: a film *multi-element shot* / a theatre stage.

**Reel + Stage compose in one world.** A Show may carry both: the Reel's morph entity and every Stage
Prop share the same coordinate space, camera, and bloom. The Reel sits at the origin by default; `reel_pos
= x,y,z` (`MARTIN_REEL_POS`) translates it, so the morphing subject is placed **relative to** the Props
— e.g. float a knot⇄galaxy morph above a Stage cityscape. The Props move with `@`, the Reel with
`reel_pos`; the camera aims at whatever world point you give it.

### Camera (`[camera]`) — the keyframed track
A **Camera** track is a list of **Keys** (was Waypoints): `t= pos= dist= yaw= pitch=`, interpolated,
with `t` either seconds or an `@@anchor`. The *kind* of move is now a first-class **`CameraMove`** enum
(`Hold | Orbit | PushIn | PullBack | Sink | Arc | Flythrough`) — film camera-move verbs in martin's
orbital space — **inferred** per segment from the pose deltas (`CameraMove::infer`) and shown in the
`MARTIN_VALIDATE` dry-run. An explicit authoring token (`move=`) and per-move easing are the deferred
next step (§9, Stage 4).

## 6. The sync bridge — anchor & cue

The bridge between Score and Show is one function, `Score::anchor_seconds()`, that resolves a symbolic
position to a time. This doc gives the two halves distinct names and a type:

- **anchor** = the *spelling* — the symbolic musical position you write (`@@drop`, `@@bar:14`).
- **cue** = the *value* — the resolved seconds on the show clock.
- **`AnchorKind`** (`Start | Section(name) | Bar(n) | Beat(n) | Seconds(f)`) now formalises the parse:
  `AnchorKind::parse(s)` builds the spelling, `Score::cue(&kind)` resolves it; `anchor_seconds` is the
  thin compose of the two. The old string-sniffing is gone.

**Rocket lineage & the north-star.** GNU Rocket addresses *values over rows*; martin addresses only
*time*. martin's `[camera]` track is already a multi-channel keyframe track (`pos/dist/yaw/pitch`) — it
is the *first instance* of a general **Track**. The full Rocket generalisation — every show knob
(`bg_dim`, `flash`, per-Shot `density`) as an addressable, keyframed **SyncTrack / Automation** — is
named here as the **north-star** and deliberately **not built yet** (§9, Stage 4). The trigger to build
it is a Showbook *engine-vraag* that needs a knob keyframed (scene-scoped looks, density dramaturgy).

## 7. The storyboard method — Showbook

The **Showbook** (`productions/<name>/SHOWBOOK.md`) is martin's load-bearing practice: **design on
paper (minutes) before rendering (tens of minutes).** Its core rule stays *"everything on screen has an
entrance and an exit"* — no static set-dressing; every Shot/Prop has a lifecycle.

**Two layers, kept distinct (the key reconciliation):**

- **Section** = the **musical time-grid** (intro/build/drop/…), drives anchors and energy. Musical truth.
- **Scene** = a **narrative beat** overlaid on 1+ Sections (camping's "Zonder deFEEST" *is* the
  breakdown; "Met deFEEST" *is* the climax).
- **Arc** = the ordered Scenes with a dramatic shape — *hook → problem → solution → CTA* for the
  intro/etalage, *exposition → dip → peak → resolution* for camping.

Laying Scenes over Sections, with rows = time and columns = channels, **is a dope sheet**. The Showbook
adopts that panel as its per-scene layout:

| Section (time) | Subject (reel shots) | Stage (props) | Camera (move) | Backdrop / FX | Music phase |
|---|---|---|---|---|---|

Kept verbatim (no prior-art conflict, all good): the **status-trap** `□ → ◪ → ▣ → ★`, the **capture
shopping-list**, and the **engine-vragen** (parked feature gaps that a `◪` scene must justify before
they're built).

## 8. Naming (UNDECIDED — shortlists)

Demoscene tradition: short, evocative, one word, slightly mythic.

### (a) The engine — **DECIDED: Martin**
The name stays **Martin**; the category is *"a deFEEST demoscene engine"* (music-synced Gaussian-splat
shows). The shortlist below is kept for the record.

| Candidate | Why |
|---|---|
| **Murmur** | *murmuration* — a flock of starlings forming shapes; literally the morph/swarm metaphor |
| **Mistral** | clouds of points blown into shape; wind/morph feel |
| **Stella / Stellae** | "of stars" — splats as a starfield condensing into form |
| **Volute** | a scroll/spiral of particles; obscure, demoscene-flavoured |
| **Cinder** | sparks reassembling; short, warm (note: also the crew's musician name) |
| **Lumen** | light/particles, clean — ⚠ clashes with Unreal Lumen |

### (b) The `.show` language
| Candidate | Why |
|---|---|
| **Libretto** | the visual "score" paired with the musical Score — the duality *is* the engine |
| **Showscript** | does-what-it-says; pairs with the `.show` extension |
| **Cuesheet** | a cue-driven, music-anchored language |
| **Reel** | the language describes reels of shots |
| **Tableau** | a staged scene; matches `[stage]`; demoscene flavour |
| **Storyscript** | ties the DSL to the Showbook |

### (c) The domain-category & the Showbook method
- **Category:** "splat demo engine" (demoscene-native) · "gaussian motion-graphics" (mograph lineage) ·
  "music-synced splat cinematography" · "realtime point-cloud VJ engine".
- **Method:** keep **Showbook** as the artifact name; describe the *practice* as
  **"cue-sheet / dope-sheet storyboarding"** so it inherits both prior arts.

## 9. Restructure roadmap (staged, smallest blast-radius first)

Aliasing is idiomatic in martin's parsers, so every DSL rename is additive and back-compatible.

- **Stage 0 — doc-only (zero risk).** This file; glossary updates in `DESIGN.md` / `AGENTS.md` /
  `ART-DIRECTION.md`; rewrite both Showbook headers to the §7 dope-sheet panel + two-layer
  (Section/Scene) framing; scrub "scene"/"transition"/"cue" ambiguity from prose.
- **Stage 1 — DSL token aliases (additive). ✅ LANDED.** Canonical tokens beside the old ones:
  `[reel]`, `[stage]`, `~entrance`, `exit:`, `flock:`, `backdrop:`, `budget=`. All shipped shows
  migrated; old spellings still parse. Plus two beyond the original Stage 1: **`kind = intro|demo`**
  (the production-kind / asset-budget check, the L1 layer) and **`[scenes]`** (the L2 arc-authoring
  layer that flattens to `[reel]`). Still pending in this stage: nothing.
- **Stage 2 — internal struct/field renames (no DSL impact). ✅ LANDED.** `Part→Shot`, `count→budget`,
  `Composed→Prop`, `Waypoint→Key`, the source cloud → `Shot.origin`. Compiler-checked, mechanical.
- **Stage 3 — small new enums. ✅ LANDED.** `AnchorKind` (`Start|Section|Bar|Beat|Seconds`) — the
  anchor *spelling*; `anchor_seconds` now = `cue(AnchorKind::parse(s))`, the **cue** being the resolved
  seconds (§6). `CameraMove` (`Hold|Orbit|PushIn|PullBack|Sink|Arc|Flythrough`) — inferred per camera
  segment from the pose deltas and surfaced in the `MARTIN_VALIDATE` dump (§5); an explicit `move=`
  token / per-move easing is the deferred next step (Stage 4), so the enum isn't a parked label.
- **Stage 4 — new concepts, on demand (highest blast-radius, last).** Per-Shot `density`;
  scene-scoped looks (per-Shot `backdrop`/`flash`/`deform`); the **SyncTrack / Automation**
  generalisation (the Rocket step). Sequenced by which Showbook engine-vraag first needs them.

## 10. Validation — the acceptance test

The model is correct **iff** the two shipped shows re-express in it with **zero expressive loss**: every
token lands in a named slot, and no concept needs a word the model doesn't define. Worked example —
`productions/intro/intro.show`:

| Show line (today) | In the model |
|---|---|
| `glb:defeest.glb @@intro @8,3 out:explode bg:off` | **Shot**: content=`glb:` · anchor=`Section(intro)` → cue 0.0s · hold 8 / morph 3 · **exit**=explode · backdrop=off |
| `splat:galaxy.ply @@build @5,2.5,1.4 ~morph ^twist bg:stars` | **Shot**: anchor=`Section(build)` · budget per-show · **entrance**=morph (from a ball **origin**) · **deform**=twist · backdrop=stars |
| `mesh:bitterbal.glb cluster:7 ~morph ^wind out:evaporate` | **Shot** with **flock**=7, entrance=morph, deform=wind, **exit**=evaporate |
| `text:code · splats · music @@breakdown ~outline ^wave raster:position` | **Shot**: entrance=outline · deform=wave · raster=position |
| `shader:bolt @@climax @1.4,0.3` | **Shot** (shader interlude) |
| `t=58.5 pos=0,0,0 dist=2.75 yaw=1.66 pitch=0.30` | **Key** on the Camera track; the surrounding pull-back+sweep → inferred **CameraMove**=Arc |
| section starts `intro 0 · build 7.7 · drop 19.4 …` | **Sections** (time-grid); the Showbook **Arc** overlays **Scenes** on them |

Every token has a home; nothing is left unnamed. **PASS.** Repeat for `camping.show` (the richest show)
to stress Props, captures, and the per-Shot density / scene-scoped-look engine-vragen — that exercise
is what flushes out the Stage-4 concepts before they're built.

## 11. Open questions

- **SyncTrack scope** — how general should Automation be (any `MARTIN_*` knob? a fixed set?), and what
  triggers building it.
- **Film's third tier** — martin's `[reel]` already *is* the sequence; is a Showbook "sequence" tier
  (above Scene) ever needed, or do Scene → Shots suffice?
- **Per-knob automation vs per-Section overrides** — the camping Showbook wants scene-scoped looks;
  does that land as Stage-4 Automation or as cheaper per-Section settings first?
- **The names** — §8 stays UNDECIDED until the author picks (optionally via `/adhd` on the engine +
  language shortlists).
