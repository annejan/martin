<!--
SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
SPDX-License-Identifier: MIT
-->

# Changelog

All notable changes to martin. Format follows [Keep a Changelog](https://keepachangelog.com/);
the project has no tagged releases yet, so everything lives under **Unreleased**.

## [Unreleased]

### Engine
- **`[camera]` track is now authoritative for *any* show** (compose-only included), no `MARTIN_FLY`
  needed. A fully-timed camera track (`t=…` on every keyframe) plays straight off the show clock in
  `flypath`; the compose/sequence auto-frame still seeds the initial pose but the track drives every
  frame, and the auto-orbit drift (`compose_camera`) + recorder front-sway stand down when a track is
  present (they'd fight the authored yaw). Before, a compose stage ignored its `[camera]` and always
  auto-framed (yaw = `FRONT_YAW`, auto-dist) unless `MARTIN_FLY` was set — so an authored/exported
  camera never took. `MARTIN_FLY` now only replays an untimed M-key waypoint path. (Reel shows with a
  `[camera]` track — e.g. cities-defeest — now follow it without `MARTIN_FLY` too.)
- Sequence engine: a timeline of *parts* (`text:` / `splat:` / `mesh:` / `glb:` / `image:` / `wall:`)
  that assemble out of a ball cloud and morph into the next, per-Gaussian on the GPU, with a directed
  camera track. Composed via `MARTIN_*` env vars or a single unified `.show` file (`MARTIN_SHOW`).
- mesh→splat sampling: density-adaptive disk size + R2 low-discrepancy distribution, per-splat
  translucency (`MARTIN_MESH_OPACITY`), and a glTF (`.glb`) loader.
- **Screen-anchored captions** (`[caption]` track): `screentext:HELLO  in @@drop out @@drop+4bar  at
  0.5,0.08  size 64` pins a title/credit to SCREEN space (a fixed screen fraction) that stays put while
  the splat camera flies — unlike `text:` (world-space gaussians). bevy_ui `Text` with
  `UiTargetCamera(orbit_cam)` so it composites into the splat camera and DOES bake into headless
  recordings (the loader screen, lacking this, doesn't). Alpha is a pure fn of the show clock
  (deterministic). `src/scene/caption.rs`.
- **Particle KINDS** (`MARTIN_PARTICLES=embers|confetti|sparks|fireworks`): the value now picks the
  effect — confetti (tumbling coloured flakes falling), sparks (hot specks bursting from a core),
  fireworks (rise-then-bloom shells), plus the original embers. All **beat-reactive** (the kick
  scatters/pops them; embers stay calm so legacy bakes are byte-identical). Deterministic CPU billboards
  (`src/particles.rs`), one shared mesh + a small palette of additive materials, RADV-safe.
- **Textured glTF → coloured splats**: the `.glb` sampler now reads each primitive's `baseColorTexture`
  + `TEXCOORD_0` and colours every splat by sampling that texture at its surface UV (`sample_texture`
  in `mesh.rs`) — so a textured model (`paard.glb` chestnut horse, `bier.glb` amber beer) keeps its
  painted colour instead of rendering flat white. Falls back to vertex colours → `baseColorFactor` →
  `MARTIN_MESH_RGB` → a pale default, as before.
- **Per-object translucency** for the composition stage: an `alpha:<0..1>` token bakes a translucency
  into that object's splat opacity (`compose.rs`) — a glass beer mug, a ghost, a haze — independent of
  the global `MARTIN_MESH_OPACITY`. The scene-wide fade-in animates on top.
- **`travel:` one-shot eased move** for stage objects: `travel:x,y,z[@anchor[,dur]]` eases an object
  from its `@pos` to a fixed target over a window then **HOLDS** — the proper "walk in and stop", unlike
  `drift` (constant, never stops) or `path:` (oscillatory). Default start = the object's `in` cue,
  default `dur` = one bar; reuses `ease:`, composes with `bob`/`spin`/`drift`/`path:` (`compose.rs`).
- **Per-shot morph easing** (`ease:<curve>`): shapes the blend curve so an entrance can LAND on the
  beat instead of always drifting in — `smooth` (default, unchanged), `snap`, `hold-snap`, `anticipate`,
  `stutter`. Pure scalar, deterministic; the single source of the morph curve now (the reel director and
  the compose stage both route their factor through `Ease::apply`, retiring the duplicated smoothstep).
- **`~shockwave` entrance**: the shape materialises as an expanding ring sweeping outward from the
  centre (a directional blast-front) instead of a uniform converge — pairs with `@@drop`+`ease:snap` so
  the kick blasts it into being. Fork transition mode 8 (radial reveal; append-only + default-off).
- **Hard camera cuts** (`cut` on a `[camera]` keyframe): the camera SNAPS to the pose at its time
  (holds the previous pose, then jumps) instead of gliding — an MTV-style editing cut on the beat.
  Honored in both timed-track and part-window samplers; round-trips through the waypoints JSON.
- **Beat-gated post-processing** (`MARTIN_POST=chroma`): a fullscreen pass over the final image that
  RGB channel-splits on the kick — the screen shears red/cyan on every drum hit. A render-graph
  ViewNode after tonemapping, so it covers the window, the live serve view, and headless recordings
  uniformly; deterministic (shear scales with the clock-driven kick), default-off, splat geometry
  untouched. The "screen reacts to the track" layer (`src/post.rs` + `assets/post.wgsl`).
- **`~cut` hard-cut entrance**: a shot replaces the previous in ONE frame at its start (no morph-in)
  with an automatic white-flash pop — an MTV-style editing cut on the beat (pair with `@@drop`).
- **`freeze:N` per-shot deform quantization**: snaps a shot's deform animation to N steps per bar so
  the wobble JUMPS on the beat (stop-motion stutter) instead of running smooth. Deterministic.
- **Anisotropic mesh splats** (`MARTIN_MESH_ANISO`): stretches `mesh:` samples into ellipsoids along
  the surface grain (the triangle's longest edge), area-preserving, so the cloud follows the mesh's
  contours instead of looking like uniform dots. `1.0` = round (byte-identical default).
- **Tonemap** (`MARTIN_TONEMAP`): default flipped `Tonemapping::None` → **TonyMcMapface** (film-grade —
  bright splats roll off instead of clipping to flat white). **The default look changed**; set
  `MARTIN_TONEMAP=none` for byte-identical legacy renders. Plus an `exposure` channel on `[sync]`
  (and `MARTIN_EXPOSURE`) — a music-timed bloom-intensity ramp, gated so the default is untouched.
- **Expression anchors**: `@@drop+2bar`, `@@drop-1beat`, `@@bar:16-3s` — any `@@anchor` ± a
  bar/beat/second offset, resolved in one place (`Score::anchor_seconds`) so reel parts, the camera
  track, `[sync]`, and compose `in`/`out` all inherit it. Lead-ins/lead-outs relative to a cue.
- cities-defeest: the NYC drop recast as the kill-shot — `~shockwave ease:snap` + `post = chroma`.
- **Onset-weighted beat reactivity**: each kick/snare/hat visual reaction is now scaled by the hit's
  metric velocity (`audio::vel` — the same accent the synth uses), so a downbeat kick punches harder
  than a ghost-note tick. The look breathes with the groove instead of every hit landing identically.
- **`[sync] fov` lens-slam**: a music-timed FOV knob (`fov=0.7` = punch in on the climax), clamped ≤1
  (only ever narrower) so the fullscreen backdrop/interlude quads stay covered.
- **`MARTIN_RES=WxH`**: the offscreen render resolution is now an engine knob (was hardcoded 1280×720)
  — `1920x1080` / `2560x1440` for a crisp compo master. Shared by record + the serve view.
- ART-DIRECTION: the "let it breathe" rule — author an empty bar before the drop (loud backdrop drowns
  the hero in a close-up; on black it glows).
- **Raymarched-class backdrops** (`MARTIN_BG=fractal` / `clouds`): a Kaliset orbit-trap (glowing
  fractal filaments) and a 6-octave fbm haze — pure 2D fragment math (iGPU-cheap, no SDF raymarch),
  deterministic, on both the backdrop + `shader:` interlude layers.
- **Visual sidechain** (`MARTIN_SIDECHAIN`): the kick ducks the whole frame (splats + backdrop) and it
  swells back — the music's pumping made visible. Scaled by beat intensity, default-off, deterministic.
- **Additive ember particle layer** (`MARTIN_PARTICLES=embers`, `MARTIN_PARTICLE_COUNT`): glowing warm
  points drift up through the scene among the splats — a second, independent motion layer. CPU-animated
  `StandardMaterial` (`AlphaMode::Add`, emissive, a generated radial-gradient texture for soft round
  glow) on real geometry — RADV-safe, since a custom additive *shader* material crashes the splat
  pipeline. Deterministic: each ember's path is a pure function of its seed + the show clock.
- **Harmonic tint** (`MARTIN_TINT_MUSIC` / `[sync] tint_music`): the backdrop palette leans cool in
  minor/low-energy passages and warm on lifts/major (from `Score::chord_at` + `gain_at`) — colour
  breathes with the harmony. Backdrop-only (a new `warmth` FxUniform field), default-off, deterministic.
- **Per-Scene camera + look**: a `[scenes]` scene header can carry `cam:` / `look:` (one token each,
  `;`-separated) that emit a `t=@@anchor` keyframe into the `[camera]` / `[sync]` tracks — a whole arc
  (content + camera + look) in one `[scenes]` block.
- **`pipeline/sheet.py`**: a contact-sheet storyboard tool — drives `MARTIN_VALIDATE` for each shot's
  time, captures a headless `MARTIN_SHOT` thumbnail per shot, and grids them into one PNG.
- **`field:N`** in `[compose]`: scatter a stage object into N seeded, randomly-rotated copies (a
  swarm/field — a plate of bitterballen, a flock of logos), the stage's equivalent of the reel's
  `flock:`. Reuses `morph::cluster_of`; deterministic.
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
- **Live control bridge** (`MARTIN_SERVE=1`, default port 7878): boot the show windowed, render
  offscreen (window-independent screenshots), and drive the camera + clock live over a line-JSON TCP
  protocol (`camera`/`seek`/`pause`/`play`/`step`/`screenshot`/`dump_camera`/`state`) — author + inspect
  **without reloading** the (possibly huge) show. `dump_camera` emits a paste-ready `[camera]` line, so
  you author the track by flying. The engine half of "full MCP".
- **MCP server** (`martin --mcp` / `MARTIN_MCP=1`): a stdio JSON-RPC MCP server (no Bevy, clean stdout)
  that proxies to the bridge, exposing camera/seek/pause/play/step/dump_camera/state as native MCP
  tools — and `screenshot` returns the PNG **inline** as image content. Registered in `.mcp.json` so an
  MCP client drives the live engine directly. Completes "full MCP".
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
  render.
- Multi-core batch synth render (~2× faster, deterministic) for recordings + the bundle WAV.
- Tracker DSL: sections/phases, per-section chords, multi-bar melody/arp/bass note-lanes, drum
  patterns, dynamics ramps, and free-form mix/fx `set` knobs.
- **Note ties** (`-`/`_` in a note lane): hold the previous note one more slot (`C4 - - .` = a held
  dotted note) — `note_line` returns `(t, freq, hold)` and the synth extends the note by `hold`. Lets
  the melody sustain instead of every note firing at the fixed slot length. Untied notes are unchanged
  (`hold == 0`), so existing tracks render byte-identically (`-`/`_` were unused rest-synonyms before).
- Per-section overrides: `<section>.set key=value` (knobs) and `<section>.fx: …` (which layers /
  transition accents fire — so a genre picks its own accents without abusing section names).
- Synth voices incl. a hardstyle kick, Reese/woozy bass, singing 5-saw lead, supersaw+choir wall,
  classic M1 "house organ", donk, casio; 2-band master, glue comp, diffuse reverb (+ section depth
  automation), Haas widening, sidechain, atmosphere bed; optional 2× oversampling (`set oversample=1`).
- **Additive instrument palette** (`leadsw` / `basssw` / `padsw` / `drumsw` knobs): alternate voice
  CHARACTERS swapped in per voice — e.g. `leadsw` 2 breathy-sung / 3 Carpenter-Brut / 4 FM-bell;
  `basssw` 2 Kavinsky-sub / 3 Brut-Reese / 4 acid-303; `padsw` 2 Juno / 3 PWM-string / 4 dark-wash;
  `drumsw` 1 analog/808 kit · 2 festival/house kit (clean punchy boom kick, no gabber tail). Knob `0` (default) keeps the original voices, so existing tracks are
  byte-identical. Resolved **per note via `param_at`**, so a `<section>.set …sw=N` line changes the
  instruments for THAT section only (softer interlude, harder crescendo). Voices in `voices.rs` (+ the
  analog kick in `effects.rs`); `*_pick` selectors in `render.rs`. The nocturnal demo runs the full
  synthwave palette; Camping keeps its bounce kit but softens the breakdown + sharpens the climax/outro.
- **Riser/transition flavours** beyond the noise uplifter (`riser`): `downlift` (a falling whoosh that
  deflates INTO a calm section — the synthwave "suck-down"), `tonalriser` (a pitched, musical riser — a
  detuned saw gliding up an octave from the chord root through an opening filter, into a climax), and
  `reverse` (an 80s reverse-reverb/cymbal swell that cuts hard on the downbeat). `<section>.fx:` tokens
  (`render_downlift`/`render_tonalriser`/`render_reverse` in `effects.rs`). Plus the 80s **gated-reverb
  snare** (`set gatesnare=1` → `snare_gated`, a big bright reverb burst cut hard at ~150 ms). The
  nocturnal demo uses them for varied transitions (reverse+noise into the drop, downlift into the
  breakdown, tonalriser + gated snare into the climax) plus a per-section pad/bass timbre arc.
- Structural lint of the score with `MARTIN_SCORE_STRICT=1` to make warnings fatal.
- Example scores showing the range: `assets/tropical.txt`, `assets/rain.txt`.
- Score split: the engine ships a **neutral** tropical-house builtin (`assets/score.txt`); each
  production owns its own arrangement (e.g. `productions/camping/score.txt`, the "Op de Camping" track).
- **Spectral reactivity** (`MARTIN_FFT`, default on): the rendered track is FFT'd into 8 log frequency
  bands (sub→air) and baked into a frame-indexed table; the background (`MARTIN_BG`) and `shader:`
  interludes react to the actual *spectrum* — bass swells the field, mids wash colour, air sparkles —
  not just the drum triggers. Dependency-free hand-rolled radix-2 FFT (`src/audio/analyze.rs`), baked
  off-thread live and synchronously in record mode, indexed by show-time → bit-identical in recordings.
  Splat-only morph shows are untouched (the coupling lives in the fullscreen-effect uniform).

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
- `pipeline/show_layout.py`: GPU-free layout preview now handles `mesh:`/`glb:` props (falls back to a
  name-based footprint instead of crashing on the missing sh0 header) and plots a `travel:` object where
  it comes to REST (its target), not its off-screen `@pos` start.
- CI: rustfmt, clippy (`-D warnings`), cross-platform build+test, REUSE, advanced CodeQL, cargo-audit.
- Dependabot (weekly) with auto-merge of green patch/minor bumps; `main` branch protection.
