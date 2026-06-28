<!--
SPDX-FileCopyrightText: 2026 Anne Jan Brouwer <brouwer@annejan.com>
SPDX-License-Identifier: MIT
-->

# Changelog

All notable changes to martin. Format follows [Keep a Changelog](https://keepachangelog.com/);
the project has no tagged releases yet, so everything lives under **Unreleased**.

## [Unreleased]

### Changed

- **Upgraded to Bevy 0.19** (was 0.18) + `bevy_gaussian_splatting` 8.0.0 (our `martin` fork rebased
  onto upstream's Bevy-0.19 release with a zero-conflict replay of our shader edits). Mechanical API
  churn handled across the engine: rodio-0.22 audio `Source` (`current_span_len`, `NonZero`
  channels/rate, `Decodable` drops `DecoderItem`), `Hdr` moved to `bevy_camera`, glTF scenes spawn via
  `WorldAssetRoot` (was `SceneRoot`), `DirectionalLight.shadow_maps_enabled`, the Parley text API
  (`Font::from_bytes`, `TextFont` `FontSource`/`FontSize`, `TextLayout::justify`), and `Assets::get_mut`
  returning a guard. Verified: 160 tests + clippy + all 34 shows render headless on 0.19. The
  `MARTIN_POST` fullscreen FX (chroma/grain/vignette screen-tear) was **ported to 0.19's "render graph
  as systems"** — the old `ViewNode` became a render-world system (`post_pass`) in the `Core3d`
  schedule's `PostProcess` set after tonemapping, fetching the view via `CurrentView` so non-`post`
  views skip cleanly. Chroma renders again (verified on campsite-max); the shader is unchanged.
- **Splat fill-rate cut (fork §9): the quad extent tightened 3.0σ → 2.4σ.** The 2.4–3σ ring of every
  splat is near-zero alpha (rasterised + blended for almost nothing); trimming it cuts the quad pixel
  area ~36 % with no visible change (it does *not* shrink the gaussians, unlike `SPLAT_SCALE`). Lifts
  every quality tier on an overdraw-bound GPU — the PonyCamp climax went **720p 30→40 fps (+32 %)**,
  854×480 → 54 fps. Gated on the SH degree (`#if SH_DEGREE > 0`): the **sh3 build = real captures keeps the full 3.0σ** (2.4σ visibly thinned an aerial-city capture — anisotropic splats need the wider tails to blend), so only the synthetic **sh0** build opts in. (`bevy_gaussian_splatting` `martin` branch, `render/gaussian.wgsl`.)
- **Live FFT-reactive visuals react from t=0 (no dead zone).** The spectrum band table that drives the
  FFT-reactive backdrop/interludes is now filled by a **causal streaming analyser** (per-band AGC) that a
  background thread appends front-to-back ahead of the playhead — instead of waiting ~9.8 s for the whole
  track to render + normalise. First rows land in ~98 ms. Deterministic / record-safe (chunk-independent;
  record still bakes the full table before frame 0). `MARTIN_FFT_NORM=track` keeps the legacy whole-track
  normalisation (and its dead zone) for byte-identical-to-old output. The startup spectrum bake also
  parallelised (rayon render + FFT).
- **Non-blocking reel build (startup-freeze fix).** A reel of big captures no longer freezes the frame
  for seconds at startup while it samples + resamples every shot. Two steps: (1) the per-part sampling
  now fans across **rayon** (each part is independent); (2) the whole heavy build moved **off the main
  thread** onto `AsyncComputeTaskPool` — a live/windowed run keeps the loader animating while it builds,
  then finalizes (GPU upload + camera framing) the frame it's ready. Deterministic capture modes
  (`--record`/`--shot`/`--bench`) build **inline before frame 0**, so a record bakes byte-identically to
  before. Verified: the camping reel renders byte-for-byte identical on both paths; all 34 shows pass
  the headless smoke test.

### Added

- **Variable grid / odd meter in the score DSL** — a section can declare `grid:N` (slots per bar,
  default 16); a slot stays a 16th-note so the bar is `N/4` beats: `grid:12` = 3/4, `grid:14` = 7/8,
  etc. The per-bar slot/note arrays became `Vec` (were `[_; 16]`) and the timeline now carries a
  cumulative `bar_slot0` table alongside `bar_secs`, so slot↔seconds + slot↔bar work with mixed-length
  bars. An odd-meter riff cycle can be one big bar (e.g. `grid:52` = |3/4 3/4 3/4 4/4|). A score with
  every section at the default 16 grid + no `tempo` line hits a `uniform` fast path and renders
  **byte-for-byte identical** to before — verified: a fresh rebuild of the previous commit and this one
  produce the same WAV md5 for the built-in score (and every shipped production score is uniform).
  Composes with tempo automation. `pipeline/midi_to_martin.py --faithful` reads the MIDI's time
  signatures (FF 58) and auto-emits one section per bar at the right grid (`--no-meter` force-fits 4/4),
  so Golden Brown plays its real 3/4↔4/4 meter and Clair de Lune keeps both its 6/8↔9/8 meter and its
  rubato (the tempo map composes on top).
- **Tempo automation (rubato) in the score DSL** — an optional `tempo @bar:N=BPM @bar:M=BPM …` line
  steps the tempo at bar boundaries (piecewise-constant per bar); `bpm N` stays the bar-0 default.
  The slot↔seconds map is now piecewise-per-bar (a cumulative `bar_secs` prefix table behind two
  funnels, `slot_to_secs`/`secs_to_slot`), so a slow bar is literally longer in seconds and a rubato
  piece (e.g. a Debussy transcription) breathes instead of sounding metronomic. **Both** the synth
  (notes, pad, drum accents) and every `@@anchor`/section time follow the map, so visuals stay
  sample-locked through a tempo change. A score with **no** `tempo` line renders **byte-identical** to
  before (the funnels keep a constant-tempo fast path). `pipeline/midi_to_martin.py --faithful` now
  emits the line automatically from a MIDI's set-tempo events (`--no-tempo-map`/`--bpm` force one
  steady tempo). Documented in `USAGE.md`; deferred (constant-tempo today, documented): tempo-syncing
  the procedural dance layers (wall/shimmer/stab/build walks) and a true intra-bar linear ramp.
- **Four tropical splatgen shapes: `palm`, `parasol`, `cocktail`, `crab`** — procedural beach props for
  the summer demo (`splatgen palm assets/palm.ply`, or `splat:palm.ply` in a show). A leaning palm with
  drooping fronds + coconuts, a red/white striped beach parasol, a martini cocktail (glass + liquid +
  straw + garnish), and a cartoon crab.
- **`pipeline/pointcloud_to_splat.py`** — convert a plain colored point cloud `.ply` (Open3D/Luma
  export: x y z + r g b) into a martin gaussian `.ply`, so a captured scene that isn't already 3DGS
  loads + morphs like any splat cloud. Robust (percentile-bbox) radius estimate + a `--radius` override.
- **`pipeline/midi_to_score.py`** — extract a monophonic melody (+ bass) from a MIDI per channel into
  martin's 16-slot tracker note-grid, so a real tune is transcribed 1:1 instead of from memory.
- **`productions/beach/`** — the "Coco Jamboo" (Mr. President, 1996) summer beach demo: a real Ham Tin
  beach capture (CC-BY lookingglass) as the world, tropical props on the eurodance drop, the hook lifted
  1:1 from a MIDI.
- **`~fade` compose props no longer double their VRAM.** A `~fade` object's "source cloud" is just its
  shape at alpha 0 (`fade_of`), so its assemble is a pure OPACITY fade — no spatial motion. It now
  renders on the plain opacity path instead of spawning a GaussianInterpolate (which held both the
  alpha-0 source and the shape resident), with the fade forced to the same 3.6 s → **byte-for-byte the
  same look, ~half the resident gaussians for a fade-heavy stage** (PonyCamp's 40-object diorama: 19
  `~fade` props, peak resident 1.61M→1.47M = lower OOM risk + a count-bound fps nudge). Genuinely
  spatial entrances (ball/scatter/swirl/…) still assemble via the interpolate.
- **`--validate` flags compose OOM risk + the `~fade` VRAM trap (GPU-free).** The dry run now prints a
  compose stage's estimated **peak resident gaussians**, warns if it's over the `MARTIN_SPLAT_WARN`
  soft cap (the OOM heads-up used to fire only once the GPU renderer was up) — catch it on paper
  before the slow render.
- **Compose `disk:` / `aniso:` now honoured (were silently dropped).** A `[stage]`/compose mesh prop's
  `disk:<size>` / `aniso:<f>` mesh-sampling tokens fell through the parser and were ignored, so a prop
  rendered at the default disk size (e.g. PonyCamp's `caravan.glb disk:0.4` drew its splats ~3× larger
  than authored). They're now parsed + threaded into `sample_content` like the reel already does.
- **Leaner build + a fast-iteration profile.** Dropped the splat crate's unused `tooling`/`viewer`/
  `web_asset` features (martin loads local files only) — **−51 crates** from the lock (a whole TLS
  stack), so a smaller binary + faster clean/CI builds, zero functionality lost. New `cargo b-fast`/
  `r-fast` profile (`[profile.fast]`, no-LTO + parallel codegen, same opt-level 3 → visually identical)
  cuts the per-edit relink **~6× (≈2m→≈18s)** for the render loop; ship + CI still use `--release`.
- **sh0 depth sort defaults to 24-bit** (was 32). Synthetic morph/text content (the sh0 default) rarely
  reorders visibly below 32, so one fewer radix digit pass is ~free fps (biggest at low res where the
  sort dominates); A/B mean pixel diff 0.22/255 (imperceptible). Real captures (**sh3**) keep 32.
  Override either with `MARTIN_SORT_BITS`.
- **`smoke-shows.py` runs shows concurrently** (`MARTIN_SMOKE_JOBS`, default 3) — each `martin` boot is
  CPU-bound, so a small pool parallelizes the full-sweep smoke test ~linearly (~45 min → a few).
- **`MARTIN_COUNT_SCALE` — global density multiplier (+ `MARTIN_QUALITY` now scales compose stages).**
  A single knob that scales EVERY resolved gaussian count — the reel part `budget` and every compose
  object's `count:`/default — so a whole scene thins even past an explicit `budget=`/`count:` that
  `MARTIN_MORPH_COUNT` can't reach. `MARTIN_QUALITY` `low`/`potato` now set it (0.5/0.4), which finally
  lets the presets thin a **count-bound compose diorama** (PonyCamp): the climax/full-party frame goes
  **39→56 fps on `low`** (854×480) and **77→88 fps on `potato`** (640×360) — both at the worst moment,
  the rest of the show faster. Default 1.0 = existing renders byte-identical. Build-time → record-safe.
- **On-screen FPS HUD (`MARTIN_FPS_OVERLAY` / `[settings] fps_overlay`, `I` key)** — the engine reports
  its own performance: a corner readout of smoothed FPS + frame-time + splat budget + the timeline clock,
  drawn over the live window (no external counter / bench-sweep needed for a quick glance). The `I` key
  toggles it alongside the existing console metric. Live windows only — never drawn into a `--record`
  video or a `--shot`, so it can't bake into a deliverable.
- **`--benchmark` auto-tuner.** Re-launches the binary once per quality tier (`potato`/`low`/`med`/
  `high`), renders the show pinned at `--benchmark-at` (default 30 s) with that tier's caps, and prints
  each tier's fps + the best one that clears `--target-fps` (default 30) — the in-process form of
  `pipeline/bench-sweep.sh`. **Caveat (documented in the output + USAGE):** each tier is timed in a
  *spawned* window, which most compositors throttle when unfocused, so an unfocused desktop / SSH session
  reads ~4× low; run it focused/fullscreen (e.g. the shipped exe on the target box) for the true rate.
- **Per-object `dscale:` compose token + `MARTIN_QUALITY=potato` tier.** `dscale:V` is a *local*
  `MARTIN_SPLAT_SCALE` (multiplies the global) so you can shrink a full-screen backdrop's overdraw-heavy
  disks without thinning the foreground props/text. New `potato` (alias `min`/`party`) quality tier
  (640×360 + 0.7 disks + `sort_bits 16` + 8k count cap) for a ~56–60 fps weak-HW floor; `low` retuned to
  854×480 + 0.8 disks (≈30 fps on a dense stage). Profiling lesson baked in: a dense multi-object stage
  is overdraw-bound at hi-res (disk size + resolution are the levers) and depth-sort-bound at lo-res
  (`SORT_BITS=16` + fewer gaussians) — and a solid splat look follows `count × disk² ≈ const`, so fewer
  splats with bigger disks beats many tiny ones (same coverage, far cheaper sort, no gaps).
- **Window mode: `--fullscreen` / `--windowed` flags + a `fullscreen` build feature.** `MARTIN_FULLSCREEN`
  is now **tri-state** — explicit `0`/`false`/`off` → windowed, any other set value → borderless
  fullscreen, *unset* → the build default. Build a **fullscreen-by-default binary** (a shipped demo /
  kiosk exe) with `cargo build --release --features fullscreen`, or bake `[settings] fullscreen = true`
  into a production's show; either is overridden at runtime by `--windowed` / `MARTIN_FULLSCREEN=0`
  (and F11/F still toggle live).
- **Live perf knobs (run on weak/party hardware).** Profiling showed the live render is overdraw- and
  splat-count-bound on a weak GPU (not a martin bug — per-cloud it matches upstream). New levers:
  `MARTIN_SPLAT_SCALE=<f>` (shrink every splat disk — cuts overdraw while keeping every splat: `0.8`
  ≈ +27 % fps, `0.5` ≈ 2.2×), `MARTIN_SORT_BITS=16|24|32` (cheaper per-frame depth sort, synthetic-safe),
  and `MARTIN_QUALITY=low|med|high` — a one-word preset stacking count + resolution (+ scale/sort on
  `low`) for **~3× fps**, set-if-absent so it caps a show's budget but an explicit knob still wins.
- **Window + profiling knobs.** `MARTIN_TITLE` (window title, also `.show [settings] title=`),
  `MARTIN_WIDTH`/`MARTIN_HEIGHT` now size the live window too, `MARTIN_VSYNC=0` (uncapped present),
  `MARTIN_BLOOM=0`, `MARTIN_HOLD_T=<s>` (pin the timeline for a deterministic sweep), `MARTIN_DIAG=1`
  (Bevy diagnostics), and `pipeline/bench-sweep.sh` (repeatable, GPU-safe count×res×scale sweep). The old
  `MARTIN_BENCH` measures CPU submit rate, not GPU completion — use the windowed `MARTIN_FPS`+`VSYNC=0`.
- **Audio die-out tail (`[score] set endfade=<s>`)** — the master fade-out length is now a score knob
  (default `0.025`, a click-guard near-hard-stop — unchanged for every existing track). A larger value
  (e.g. `1.6`) lets the final impact + reverb **ring out into silence** instead of the mix hard-cutting
  at the last sample. The camping lyrics demo uses it for a real explosive die-out.
- **`MARTIN_FADE_OUT=<s>` (record.sh)** — the video fade-to-black length at the end of a render (default
  `2.6`). A show with a late visual climax (an explosion that must stay lit through the final bang) sets
  a shorter tail so the picture doesn't fade out *over* the payoff. Read by the mux step directly (the
  `.show` `[settings]` block only reaches the martin binary, not ffmpeg — pass it on the record.sh CLI).
- **"Op de camping" lyrics demo (`productions/camping/op-de-camping-lyrics.show`)** — a simple
  karaoke cut: all of Ome Henk's 1995 lyrics on screen as a timed `[caption]` track over a lean
  morphing-splat backbone with section-cued backdrops, ending on a **real explosion** — the splat
  bursts apart (`exit:explode`) into a trailing `~shockwave` blast with a `bolt` lightning backdrop
  on the outro bang, then dies out to black (the buildup finally pays off instead of resolving into
  nothing).
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

### Fixed

- **A single `[camera]` keyframe now drives the camera (held static pose).** An inline `[camera]` track
  with exactly one timed keyframe was silently ignored — the `is_track` gate requires ≥2 keys, so a lone
  pose fell through to the `build_*` auto-frame and the authored camera was lost (a sharp authoring
  footgun: a one-shot `.show` camera looked like it "didn't apply"). The flypath now applies any **inline
  timed** camera (`is_timed`, ≥1 key): one key holds that pose for the whole show (`pose_at_time` returns
  it for all `t`), ≥2 still interpolate as a music-timed track. M-authored *files* stay gated by
  `is_track` (≥2) so a second live-stamped waypoint doesn't snap-hijack free-flying. `--validate` now
  labels the three cases: `track (music-timed)` / `held pose (single keyframe → static camera)` /
  `path (untimed → inline keys are ignored)`.
- **Captions: a lyric word that is also an option keyword no longer truncates the line.** The
  `[caption]` parser split text from options at the *first* keyword token, so a bare `in`/`at`/… inside
  the text (e.g. "dooie beessies **in** me thee") cut the caption short and desynced its options. It
  now splits at the first keyword that begins a *valid* trailing-options run, so stray keyword words
  stay part of the text.
- **Centred captions now centre multi-line text** (`Justify::Center` + edge padding) instead of a
  wrapped second line hugging the left edge.

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

- **Catmull-Rom camera (`MARTIN_CAM_SPLINE` / `.show` `cam_spline=1`)** — opt-in: interpolate the
  `[camera]` track THROUGH its keys with continuous velocity (a flowing, never-stopping glide) instead
  of the default per-leg smoothstep that settles at each key. `cut` keys still snap; neighbours clamp at
  the ends + don't cross a cut. Off by default → existing shows unchanged.
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

- **`record.sh` synth cache** — the synth is a pure function of the score, so a visual-only iteration
  (same score, tweaked `.show`) now reuses the rendered WAV and skips the ~18 s re-synth on every
  preview. Keyed on the resolved score file's hash, cached under `~/.cache/martin-synth/`;
  `MARTIN_NO_SYNTH_CACHE=1` forces a re-render (e.g. when overriding a synth param via env).
- `pipeline/show_layout.py`: GPU-free layout preview now handles `mesh:`/`glb:` props (falls back to a
  name-based footprint instead of crashing on the missing sh0 header) and plots a `travel:` object where
  it comes to REST (its target), not its off-screen `@pos` start.
- CI: rustfmt, clippy (`-D warnings`), cross-platform build+test, REUSE, advanced CodeQL, cargo-audit.
- Dependabot (weekly) with auto-merge of green patch/minor bumps; `main` branch protection.
