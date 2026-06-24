<!--
SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
SPDX-License-Identifier: MIT
-->

# Changelog

All notable changes to martin. Format follows [Keep a Changelog](https://keepachangelog.com/);
the project has no tagged releases yet, so everything lives under **Unreleased**.

## [Unreleased]

### Added
- **Cinematic post-FX (`MARTIN_POST=cine` / `grain` / `vignette`)** — the post pipeline grows beyond
  chroma: film **grain** (a deterministic per-pixel+frame hash, record-safe) and a **vignette**
  (darkened corners), composable with `+` (`chroma+vignette`) and scaled with `:<strength>`. The
  **`cine`** preset = chroma+grain+vignette for an instant film look. `mode` is now a bitfield;
  existing `chroma` shows are unchanged.
- **`particles=petals` — cherry-blossom rain** — a new particle kind: pink/white petals drifting +
  tumbling slowly down the frame (aliases `blossom`/`sakura`), beat-nudged. A generic "rain of …"
  layer any production can use (the Turning Japanese demo rains blossoms). Additive — existing kinds
  (embers/confetti/sparks/fireworks) unchanged.
- **`particles=meatballs` — falling bitterballen** ("cloudy with a chance of meatballs", aliases
  `bitterballen`) — the first MESH particle kind: real `bitterbal.glb` scenes rain + tumble in 3D
  (not glow sprites). Since the demo scene is all emissive splats/shaders with no light, the mesh-rain
  spawns its own key + fill directional lights so the lit balls are visible. Count capped lower (meshes).
- **Koto voice (`leadsw=6` / `arpsw=4`)** — a plucked oriental string (bright metallic attack, fast
  string decay, a tiny pluck-bend) for Japan-themed melodies. Drives the **`productions/tj/`** cover of
  The Vapors' "Turning Japanese" (koto lead + high koto arpeggio, analog drum kit, warm red/white
  Japan-abstract `.show`: rising sun, Hokusai wave, blossom bursts). Composition credited in REUSE.
- **More synth voice characters (generic, additive)** — the voice-switch palette grows so any score can
  pick its sound: `arpsw=2` an FM **bell**, `arpsw=3` a bright **glass pluck**, and a new **`stabsw`**
  for the chord-stab voice (`1` a fat **rave-organ**, `2` a bright **saw stab**). All default to the
  original voices (`0`) → existing tracks byte-identical; benefits every production.
- **Pan-flute arp voice (`arpsw=1`) + arp gain knob (`set arp=`)** — a breathy hollow flute (sine +
  soft harmonics + highpassed breath noise + a chiff attack + swelling vibrato) for melodic lines on
  the arp lane; the arp lane now respects a `set arp=<gain>` level (was a fixed 0.20). Used for the
  Dance 2 Trance "American Natives" flute chant. Part of the growing voice-switch palette
  (`leadsw`/`basssw`/`arpsw`/`padsw`/`drumsw`).
- **`productions/d2t/`** — a Dance 2 Trance "Power of American Natives" (1993) cover: the score
  transcribed from a public MIDI (`pipeline/transcribe_midi.py`, now with an `--arp` lane), the flute
  chant on `arpsw=1`, lead on the hoover, trance supersaw `wall` + house stabs; a warm desert/native
  abstract `.show` (sun/dreamcatcher/spirit-fire morphs, FFT-reactive sky). Composition credited in REUSE.
- **Hoover lead voice (`leadsw=5`)** — the iconic Alpha-Juno "What the…"/Mentasm rave lead: a thick
  detuned saw stack + a fifth + a sub-octave through a resonant low-pass, with the signature short
  downward pitch-dive on attack + tanh drive. Plus **`pipeline/transcribe_midi.py`**, a MIDI→tracker-DSL
  transcriber (the inverse of `score_to_midi.py`), used to start a U.S.U.R.A. "Open Your Mind" rave cover
  (`productions/usura/`, hoover approximates the vocal hook — no sampler in the synth).
- **`baby` procedural splat shape** — a chubby cartoon baby (round skin head with dark eyes, rosy
  cheeks + a little mouth, a hair curl, pastel-blue onesie body, stubby arms/legs) synthesized by
  `splatgen`/`build.rs`. Faces +Z; head rides on top after the load-flip. Drives the new
  **`productions/baby/`** demo (`anthem-baby.show`): the baby pirouettes on a pedestal of light to the
  actual Alexandrov USSR/Russia anthem melody — transcribed note-for-note from a public MIDI and
  quantised to the tracker grid (`productions/baby/score.txt`), in our own synth hymn arrangement.
- **`font:<name>` for `text:` parts** — pick the glyph font for filled text (`text:` + `~outline`).
  Adds the blocky **deFEEST** display font (`font:defeest` / `brand` / `hardpixel`) alongside the
  default bold font, for branded titles (e.g. PonyCamp's `text:PonyCamp ~outline font:defeest`).
  `~pen-write` still traces the single-line stroke font. The deFEEST font (Hardpixel-based, free for
  personal use) is bundled + REUSE-annotated.
- **`caravan.glb`** asset (PD/CC0) — a camper for the PonyCamp camp scene.
- **`record.sh` disk pre-flight** — a full 60 fps PNG dump is many GB and used to overflow a RAM-backed
  `/tmp` (or a near-full disk) MID-render with a cryptic "Disk quota exceeded (os error 122)" that looks
  like the shell broke. `record.sh` now reports the scratch space + per-frame size + how many frames/
  seconds fit, aborts before the slow build below a hard floor (`MARTIN_DISK_FLOOR_GB`, default 3), and —
  once the synth pass reveals the show length — aborts before the long capture if the dump won't fit,
  with the `TMPDIR=…` / lower-fps/res hint. Catches the overflow up front instead of failing half-way.
- **GPU-OOM heads-up (`MARTIN_SPLAT_WARN`, default 2.0M)** — at build martin estimates a scene's PEAK
  resident gaussians (every reel shot's shape + `~entrance`/`exit:` source clouds, plus every compose
  prop ×2 for an `~entrance`) and logs a `WARN` if it exceeds the soft cap. The dev iGPU renders ~2M
  fine but OOMs ~2.5M — a long record would die mid-render with a wgpu buffer *Validation Error* (no
  panic, frames just stop). The warning fires before the slow render so you can lower
  `MARTIN_MORPH_COUNT` / the `.show` `budget` first. Pure heads-up; never clamps or changes output.
- **`rocket` / `saturn` / `ufo` procedural splat shapes** (`splatgen`) — a space set:
  - `rocket` — a cartoon rocket: white hull with red bands, a red nose cone, three swept tail fins, a
    cyan porthole, and a hot exhaust plume (the plume reads as translucent fire like `flame`). Nose
    points +Y, so a `travel:` toward +Y reads as a launch.
  - `saturn` — a banded cream gas giant with a wide tilted ring system.
  - `ufo` — a flying saucer: a metal disc, a cyan glass dome, and a ring of alternating rim lights.

  The RUIMTE show now flies rockets + a UFO through the drop + climax, with `saturn` and a cratered
  `moon` hanging in the void, so the cosmic journey reads as *space*, not just abstract morphs.
- **`MARTIN_MORPH_STAGGER` / `.show` `morph_stagger =`** (0..1, default 0) — per-particle staggered
  morph timing. At 0 a morph slides the whole cloud in lockstep (straight-line streaks); higher spreads
  each splat's transition over its own sub-window so the cloud **dissolves + reforms** — a soft, cloudy
  transition. Drives a new `morph_stagger` uniform in the splat fork (§8). The cities-deFEEST show uses
  `morph_stagger = 0.6`.
- **`tint:white`** (alias `albino`/`snow`) — an albino recolour: near-white with a soft top-down
  falloff so a textured asset reads as a pure-white animal (the *shape* carries it). Used for the
  white peacock in the camping demo.
- **Camping demo barnyard** — `productions/camping/campsite-max.show` gains two grazing goats
  (`geit.glb`, build) and a pair of peacocks (`pauw.glb`): a colourful one (drop) and a rare white one
  (`tint:white`, drop), strutting front-of-camp to finish off PonyCamp.

### Removed
- **Pruned ~23 never-used global `MARTIN_*` env vars** — mesh-sampling (`MESH_COUNT`/`MESH_SPLAT`/
  `MESH_THIN`/`MESH_OPACITY`/`MESH_RGB`/`MESH_JITTER`/`MESH_ANISO`/`MESH_RANDOM`), image
  (`IMG_STRIDE`/`IMG_SPLAT`/`SVG_PX`), camera (`TONEMAP`/`EXPOSURE`/`AABB`/`NORMALIZE`), reel
  (`REEL_POS`/`PAIR_COLOR`/`TRANSITION`/`DEFORM_AMP`/`DEFORM_SPEED`), standalone-glb (`GLB_POS`/
  `GLB_SCALE`/`GLB_DIST`), and the legacy direct-content path (`TEXT`/`PLY2`/`REFORM`). None were set
  by any production/show/CLI. The values became fixed sensible defaults (zero behavior change); the
  surviving authoring knobs are the `.show` `[settings]` keys + per-part tokens (`disk:`, `~name`,
  `^name:amp`). `MARTIN_PLY` stays as the asset-root setting; `MARTIN_GLB` still loads a standalone
  splat scene (now origin-centred, camera auto-frames).

### CLI
- **A real CLI** (the start of moving config off the ~85 global `MARTIN_*` env vars): `martin [SHOW]
  [--record DIR] [--shot PATH --shot-at S] [--shots T1,T2] [--bench N] [--validate] [--strict]
  [--serve [PORT]] [--synth-wav PATH] [--dump-score PATH]`, plus `--production NAME` →
  `productions/NAME/NAME.show`. Each flag compiles to its `MARTIN_*` env var with **overwrite**, so the
  precedence is **CLI flag > env > `.show` [settings] > default**. Every env var is still honored
  (record.sh / CI / the bundle unchanged). (clap.)
- **`martin mcp` subcommand** replaces the `--mcp` flag (`.mcp.json` updated) — clap owns argv, so the
  pre-parse `--mcp` special-case is gone from `main`. `$MARTIN_MCP` still works for parity.

### Engine
- **Free source captures after build** — the raw `.ply` clouds (held alive until the reel is built)
  are now dropped once sampled into the shots, so a big multi-capture demo doesn't keep the huge source
  files (e.g. 4 city `.ply` ≈ 1.5 GB) in RAM on top of the built shots. Pairs with splat streaming.
- **Splat streaming — windowed VRAM residency for reels (core)** — a morph reel used to upload EVERY
  shot's cloud to the GPU at once, so N big captures × the budget blew past the iGPU's ~2.5M-splat
  ceiling (a 6-city skyline at full density = OOM). Now each `BuiltShot` keeps its cloud on the CPU
  (RAM is cheap) and the director uploads only the **active window `{idx-1, idx, idx+1}`** as GPU
  assets, dropping the rest (freeing VRAM) as the timeline moves. Peak resident drops from *all shots*
  to ~3, so per-shot density can go far higher (the cities jump from ~260k to ~600k+ each) and long
  reels of large captures no longer OOM. Foundation for high-res capture content. `model.rs` (CPU
  master clouds + lazy handles), `build.rs` (`ensure_window`), `director.rs` (stream per shot change).
- **`MARTIN_SIDECHAIN` now pumps the composition stage too** — the visual sidechain (the kick DUCKS the
  frame, the classic pump) was reel-only; `animate_composition` now honours it as well, so every placed
  prop breathes with the track. One duck factor × each prop's opacity; `0` = off (byte-identical). The
  camping demo (`campsite-max`) turns it on (`sidechain=0.5`) alongside a punchier `cam_pump=0.30`,
  `fft=1.6` (harder FFT-reactive backdrops), and hard beat-CUTS on the drop/climax camera waypoints.
- **`aniso:<f>` — anisotropic mesh splats** (reel per-part token): `>1` stretches each sampled `mesh:`
  splat into an **ellipsoid along the surface grain** (the triangle's longest in-plane edge),
  area-preserving, so the cloud follows the mesh's contours for a streaky/painterly read instead of round
  dots; `1` (the default) = round disks, byte-identical to before. Restores the math removed with the
  global `MARTIN_MESH_ANISO` env var, now a per-part token. `mesh.rs`, `content.rs`, `model.rs`,
  `parse.rs`.
- **`^name@morph` — morph-gated deform** (reel): append `@morph` to a per-Shot `^deform` and its
  amplitude becomes a half-sine over the morph-in (0 at the ends, peak at the midpoint, 0 through the
  rest of the hold) instead of running the whole time. A wobble that builds as the shape transforms then
  goes limp — the guinea cavia **writhes/boils as it cooks** into the cuy, then settles. `director.rs`.
- **Slab re-orient for extruded-logo splats** (deFEEST): ~half an extruded logo's triangles are
  edge-on extrusion walls → sampled as round disks they become slivers speckling the outline. Detect
  the slab axis (dominant normal, power-iterated) and, for slab-like meshes only (self-gating), face
  every disk along it → the walls fill the stroke as clean front-facing disks. `src/mesh.rs`.
- **Jittered mesh sampling (no weave)**: the R2 low-discrepancy sample sequence is even but *regular* —
  a faint grid weave shows in dense fills. Surface sampling now adds a deterministic per-sample jitter
  (~one local cell) — "jittered low-discrepancy": R2's even coverage without the grid pattern, and
  without pure random's clumping (A/B confirmed: random blotches, R2 weaves, this is clean).
- **Crisper mesh→splat edges (less fray)**: surface sampling now sizes each triangle's disks to its
  OWN local sample spacing (a thin logo-outline triangle no longer gets the mesh-wide-average disk that
  overhung its colour across the seam), and **edge-insets** samples toward the triangle centroid so a
  disk near a colour boundary doesn't straddle it. Sharp 2-colour graphics (the deFEEST logo) read
  markedly tighter; organic textured meshes are unchanged. `src/mesh.rs::sample_surface_disks`.
- **Anti-aliasing for the master**: `MARTIN_SS=2` (in `record.sh`) renders at 2× `MARTIN_RES` and
  lanczos-downscales in the mux — supersample AA that noticeably smooths the splat disk-edges + text
  (there is no in-engine AA). For the final master (≈n² fill), not fast previews.
- **`MARTIN_AABB`**: render splats as their true projected ellipse (conic) instead of a round OBB blob.
  Subtle for synthetic normal-oriented disk-splats; kept as a gated lever (default off) for real captures.
- **Correct glTF mesh colours**: `baseColorFactor` + glTF vertex colours are LINEAR per spec, but the
  render shader decodes every DC colour as sRGB → they were double-decoded (dark / desaturated, e.g. the
  deFEEST logo's blue+yellow). Now sRGB-encoded before the SH DC encode so they render correctly
  (textures + text/`MARTIN_MESH_RGB` are already sRGB → unchanged). `src/mesh.rs` `lin_to_srgb`.
- **Headless `MARTIN_SHOT`/`MARTIN_SHOTS`**: single screenshots now run truly headless (no window) like
  the recorder — render the camera to an offscreen image instead of the OS window (which renders black /
  panics acquiring its swapchain on RADV when unfocused), and **wait for the file to land** before
  exiting. A 1080p frame in ~5 s vs a full video render — fast iteration / profiling. (`MARTIN_BENCH`
  also confirms splat COUNT is not the per-frame bottleneck: ~350 render-fps at 50k and 400k alike.)
- **Per-part mesh disk size** `disk:<f>`: a reel-part token setting the splat-disk overlap factor for
  that part only — smaller = crisper edges on a sharp graphic without blurring other parts.
- **Smoother textured splats**: glTF `baseColorTexture` sampling in the mesh→gaussian path is now
  **bilinear** (was nearest-texel) — textured `mesh:` objects read the texture smoothly between texels
  instead of stepping, so they no longer look blocky/low-res. (Trilinear/mipmaps would only help under
  heavy *minification* — when the texture out-resolves the splats — which dense splat clouds avoid;
  skipped as diminished returns.) Pair with a higher `budget` for crisp hero objects.
- **Reel parts can sit on a surface**: a new `ground:<y>` part token seats a part's **lowest splat at
  world-Y `<y>`** (e.g. on a `[stage]` plate) instead of centring it on the origin — shape-independent,
  so a tall animal and a flat dish both rest on the same plate; omit it to let a part float. Computed in
  the **world frame** (it accounts for the cloud's baked 180°-X Y-flip — grounding the local cloud would
  seat the ceiling). See USAGE “Per-part rotation/cluster/ground” + ART-DIRECTION “Object orientation &
  positioning” (new section documenting the per-asset `rot:` gotchas, the Y-flip, and `mesh:` vs `glb:`
  for textured morphs).
- **Faster big-cloud morphs**: the build-time `pair=match` pairing (`match_reorder`) now splits large
  clouds into K equal x-slabs and matches them in parallel (rayon). A 1.2 M-splat city↔city pairing
  drops from **~18 s to ~0.45 s** per transition (~40×) — the cities loader pause goes from tens of
  seconds to ~1 s. K is derived from the splat count (not CPU count), so the result stays identical
  run-to-run; small clouds keep the exact serial path. No visible change to morph flow.
- **Localized particles**: `MARTIN_PARTICLE_ORIGIN=x,y,z` re-centres the particle field and
  `MARTIN_PARTICLE_SPREAD` scales it — a single value OR **per-axis `x,y,z`** (e.g. `1.6,0.8,1.6` for a
  WIDE, shorter campfire spread). Settings `particle_origin=`/`particle_spread=`; default (0 / 1)
  unchanged. The **embers shape** is now a campfire PLUME (wide base → rises tall → fizzles to 0 near
  the top, no hard block-end). Authorable in Blender via a `PARTICLE.embers` empty (location → origin,
  scale → spread) — `pipeline/blender_bridge.py` imports/exports it.
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
