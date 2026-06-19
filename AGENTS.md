# Op de Camping

## Philosophy — this is the DEMOSCENE: spend the CPU, dazzle the eye
martin is a demoscene engine. **Maximalism is the goal, not restraint.** Default toward MORE effects,
more layers, more morph budget, richer motion — the wow factor wins.

The hardware shapes where to spend: this box is an **AMD Radeon 860M iGPU + a 16-core Ryzen** (see the
dev-hardware memory). **The iGPU is the bottleneck; the CPU is abundant.** So:
- **Don't pre-optimise for memory, processes, or CPU time — all three are free here.** Use huge buffers,
  spawn processes/threads freely, burn cycles. Heavy CPU precompute (sampling, nearest-match pairing,
  baking, multi-pass generation) is *free real estate* — use the 16 cores. Parallelise with rayon/threads
  rather than shrinking the work. A 30s one-time build cost for a gorgeous render is a good trade.
  **We are currently maxing out ONLY the GPU** — RAM and CPU have headroom to spare; reach for them.
- **The GPU budget is the real constraint** — splat overdraw, fill rate, sort cost. That's where to be
  thoughtful (e.g. grazing camera angles through dense clouds can wedge RADV; budget vs fps).
- Caveats about "expensive" or "memory-intensive" CPU work are *not* reasons to hold back here. If an
  effect needs a big precompute, build it. Spend cores like they're going out of style.

Recording is offline (headless, deterministic) — there is *no* real-time limit on the CPU side there at
all; the only ceiling is the per-frame GPU time. Lean into that.

## Goal
Transform the "Op de Camping" (Ome Henk, 1995) demoscene track from a basic placeholder into a full-spectrum audio experience. All melodic lines (lead, arp, bass) must remain unchanged — only drums, dynamics, spatiality, and camera are free to modify.

## File Structure
- `assets/score.txt`: Tracker DSL — BPM, sections, chords, pattern tables, per-drum-lane hit patterns, gain/sub/mids curves. Editable at runtime (no recompile).
- `src/audio/`: FunDSP voice synthesis — kick, snare, hat, lead, arp, bass, stab, pad, sub, reverb, sidechain. Split into `mod.rs`, `voices.rs`, `effects.rs`, `render.rs`, `stream.rs`, `analyze.rs`. Master is the `MasterChain` in `src/audio/stream.rs` (one streaming engine). Requires recompile.
- `productions/camping/camping.show`: Unified show format — camera track, text sequence with @@anchors to score sections.
- `src/score/`: Parser for score.txt — resolves section timing, drum hits, note lanes, phase lookup. Split into `types.rs`, `parse.rs`, `dump.rs`, `validate.rs`, `mod.rs`.
- `src/music.rs`: Bevy plugin binding between score and audio playback.

## Build & Render
- `cargo run --release` runs the full Bevy app (with visuals).
- `MARTIN_SYNTH_WAV=<path.wav> cargo run --release` renders WAV only (headless, no visuals).
- `ffmpeg -i <input.wav> -b:a 320k <output.mp3>` converts to MP3.
- Typically ~228s at 44.1 kHz, ~3.5 min track.
- `./record.sh out.mp4` captures the timeline to mp4 (headless PNG dump → ffmpeg).
  - **Full 60fps dumps ~5300 PNGs (~10 GB) → overflows the RAM-backed `/tmp` tmpfs** (14 GB here),
    failing mid-render with `Disk quota exceeded (os error 122)`. Point the scratch dir at a real
    disk: `TMPDIR=/home/<you>/.cache/martin-render ./record.sh out.mp4`.
  - `MARTIN_PREVIEW_FPS=8` (low-fps preview) + a small `MARTIN_MORPH_COUNT` keeps test renders fast
    and small enough for `/tmp`.

## Score DSL (score.txt)
- `bpm <bpm>` 
- `chords: <chord list>` — global chord progression, cycles per bar.
- `section <name> <bars> <phase-bars,csv> [fill]` — defines sections. Phase bars are comma-separated (e.g. `8,8,8,8 fill`). `fill` means the last bar is a fill.
- `section.chords:` — per-section chord override.
- Per-section drum patterns: `<section>.kick|snare|hat|stab p0|p1|p2|p3|fill: <16 steps (x=hit .=rest)>`
- Per-section melodic lanes: `<section>.lead|arp|bass p0: <16 notes per bar, multi-bar phrase>`
  - Melody loops continuously ignoring drum phase boundaries (uses only p0).
  - Drum phases look up by index: `phase_at(bar_into_section)` returns which phase (0,1,2,3,255=fill).
  - Undefined phase = silence `[false; 16]`.
- Dynamics: `gain|sub|mids <section> <value_or_ramp>` — e.g. `gain build 0.25>1.1` ramps across section.

## Audio Architecture (src/audio/)
- **Kick**: `render_hardkick` — hardstyle/gabber approach: pitched sine body sweep + tonal tail on chord root + click transient. Rendered to own buffer (never ducked by sidechain). Level: `0.92 * (0.9 + 0.1 * vel(...))`.
- **Snare**: Enhanced with clap body layer (280 Hz low-passed noise, slower decay). ±0.2 pan spread.
- **Hat**: Enhanced with body layer (3.5 kHz noise layer). ±0.65 pan spread.
- **Lead**: Main melody. Was 0.34 → bumped to 0.50 (currently 0.55). Octave sheen at 0.15 (was 0.12). Climax extra sheen at 0.18 (was 0.14). Haas widening (12 ms R-channel offset, 600 Hz–6 kHz band-limited).
- **Arp**: Ping-pong delay (8th-note, bounces L-R, separate buffer). ±0.7 pan. Arp volume 0.20 (was 0.15).
- **Bass**: Kick reinforcement + articulated bassline on top of continuous sub drone.
- **Pad**: Auto-pan (slow LFO, 0.4× BPM sine).
- **Stab**: Chord spread ±0.75.
- **Reverb**: Wet 0.35, comb feedback 0.88.
- **Sidechain**: Depth 0.7, recovery 0.08 — triggered by kick buffer.
- **Limiter**: Threshold 0.93 with soft-clip. Gain > 1.0 engages compression.
- **Mid-side**: Widening factor 1.55.

## Section Structure
- intro: 8 bars (single phase)
- build: 17 bars (8,8 fill) — verse (G-minor)
- drop: 33 bars (8,24 fill) — chorus (G-major)
- breakdown: 17 bars (8,8 fill) — verse (G-minor)
- climax: 33 bars (8,8,8,8 fill) — chorus (G-major), p3 = fake-out breather
- outro: 25 bars (8,8,8 fill) — chorus (G-major), escalating finale

## Key Decisions
- Melody is sacred — comes from MIDI transcription of the actual song, never change lead/arp/bass note data.
- Fills are one bar of max intensity — intentionally over the top.
- DnB two-step foundation: kick on 1 + "and of 3" (step 11), snare on 2 & 4 (steps 5 & 13).
- Section gain curves should have wide contrast (intro quiet → drop/climax/outro pushing limiter).
- Sub bass should drone through breakdown (sub ~0.55, gain ~0.07, mids ~0.08).

## Visuals & Camera
- Show file controls camera position/movement and text overlay sequence.
- Camera was changed from static `pos=0,0,0` to 3D movement: arcs, flybys, push-ins, blast-back on finale.
- Camera responds to kick beat-pump (slight scale/position pulse).
- **SCENE DESIGN LANGUAGE — read `ART-DIRECTION.md` § "Scene design language" before composing any scene.**
  The hard rules: no object overlap · mind the Z (backdrop sorts BEHIND props or they vanish — push its
  centre back) · ground props · soft per-object density (size/opacity baked per shape, `count:` override) ·
  rule-of-thirds (never dead-centre). Vet EVERY scene in `pipeline/show_layout.py` (GPU-free, <1 s:
  overlap/grounding/rim/occlusion/3D/screen-thirds) BEFORE rendering. Iterate: python tool → low-res
  `record.sh` preview → full quality last. SEND renders to the user (don't just inspect).

## Blender ↔ martin bridge (MCP scene-making)
Author/inspect a scene visually in Blender via the `blender-mcp` MCP server (`uvx blender-mcp` + the
Blender add-on, port 9876; `blender` is symlinked to `blender-5.1`). Round-trip: drag a prop/camera in
Blender → read it back → write `[stage]`/`[camera]` → render. **Calibrated, exact:**

- **Engine normalize** (`morph::normalize_to`, every part): centre = **centroid mean**; scale =
  `NORMALIZE_EXTENT*0.5 / p90` = **1/p90** (90th-pct distance from centroid). Match this in Blender or
  positions won't line up.
- **Coord map** martin world (Y-up) ↔ Blender (Z-up): `blender = (wx, -wz, wy)`; inverse
  `world = (bx, bz, -by)`. A `[stage]` `@pos` IS a world position (the normalized cloud's centroid sits at
  the world origin), so **export prop**: `@(bx, bz, -by)`, `*scale = blender_max_dim / 2`.
- **Display a splat `.ply` in Blender**: parse the Brush SH-3 header (count `property float` lines → stride),
  cols `x,y,z` + `f_dc_0..2`; `rgb = clip(f_dc*0.2820948 + 0.5, 0, 1)`. Build a `bpy.data.pointclouds`
  (`pc.resize(n)`; set `position`/`radius`/`color` FLOAT_COLOR attrs), emission material reading the
  `color` attr, viewport `shading.type='MATERIAL'`. City point n shows at `blender = (nx, nz, -ny)`.
- **Camera** (martin orbit = target+dist+yaw+pitch): from Blender cam world `P` and its look target `T`
  (both →martin): `dist=|P−T|`, `d=(P−T)/dist`, `yaw=atan2(d.z, d.x)`, `pitch=asin(d.y)`, `pos=T`. Give the
  Blender cam a Track-To an empty at the city centroid so `T` is stable. **FOV (load-bearing):** martin uses
  `FRAC_PI_4` (45°) **vertical** FOV (`camera.rs update_fov`, Bevy default; the `[sync] fov` knob scales it).
  Blender's 50 mm default (~27° vertical) is far too narrow → set `cam.data.sensor_fit='VERTICAL'`,
  `cam.data.angle=π/4`, render 16:9 — then framing (zoom + angle + positions) matches martin. VERIFIED:
  3 colored cubes land green-left / blue-top / red-right in both Blender render and martin.
- **Text orientation**: a Blender text placeholder needs just `rotation_euler=(90°,0,0)`, **positive scale,
  NO mirror** — it then lands in the world XY (z=0) plane exactly like martin builds `text:` (`text.rs`:
  +X right, +Y up, Y-down→base-flip), so Blender camera-view = martin. VERIFIED (hi-res Blender shot +
  martin render agree: "deFEEST" reads d-left, no flip). Earlier `(90,0,180)`/X-mirror notes were WRONG —
  an artifact of misreading low-res chunky-font screenshots; the map is a proper rotation (det +1) so
  nothing needs reflecting. Real props (`splat:`/`mesh:` `.ply`/`.glb`) loaded through the same point-map
  come out correct automatically — no per-prop orientation fixes.
- **Deliver renders**: `DISPLAY=:0 xdg-open <png>` (SendUserFile isn't always available). Renders stay
  low-Q (see Common Pitfalls / quota); splat **budget** can go high for a crisp still (it's frame *count*,
  not splat count, that fills the disk).

## Testing
- `cargo test` — 54 tests pass.
- `MARTIN_SCORE_DUMP=<path>` writes normalized score dump for debugging.

## Common Pitfalls
- **Props vanish at steep/top-down angles** = the backdrop splat cloud sorts over them (per-cloud depth sort,
  no cross-cloud z-buffer). Push the backdrop centre back so it sorts behind everything (see ART-DIRECTION § Z).
- **Everything dead-centre** = no craft. Frame for the thirds (`show_layout.py` prints screen position).
- **Plastic/dense props** = too many opaque splats; drop `count:` + the per-shape opacity, don't just lower alpha.
- **Spawning multiple martin renders at once** wedges the GPU — render ONE at a time, tiny budget; use the python tool for iteration.
- Changing phase bars (`8,8` → `4,4,8`) without adding corresponding p0/p1/p2 drum patterns = missing patterns = silence.
- Changing hat patterns from swung to straight kills the DnB groove.
- Breaking the fill bar count (phases sum + 1 fill must equal section bars).
- Missing `p1` patterns in sections that use them (build uses p0+p1, drop uses p0+p1, breakdown uses p0+p1, climax uses p0+p1+p2+p3, outro uses p0+p1+p2).
