# martin — usage & env vars

`martin` loads Gaussian splats and flies a camera around them as they **morph into one
another**. Compose a show by combining env vars on the command line, or gather a whole show
into a single [`.show` file](#the-unified-scene-file-martin_show) (`MARTIN_SHOW=x.show`) — the
file just expands into the same env vars, so everything below applies either way.

```bash
cargo +nightly run --release        # nightly toolchain is pinned (rust-toolchain.toml)
```

It's **one sequence engine**: every run is a list of *parts* (splats, text, meshes, shader
interludes) that each assemble out of a source cloud and morph into the next. With no env vars
it plays the **intro production** ([`productions/intro/intro.show`](productions/intro/intro.show))
— the same show CI bundles; the older effect-catalogue demo is [`assets/demo.show`](assets/demo.show).

---

## The show is a sequence (shorthands build one)

Everything is one timeline. `MARTIN_SEQ` writes it explicitly; the other env vars are
**shorthands** that build a sequence for you (first match wins):

| If you set… | The sequence it builds |
|---|---|
| `MARTIN_SEQ` | exactly the parts you write (the full timeline) |
| `MARTIN_TEXT` | one part: that title, assembled from a ball |
| `MARTIN_PLY` (+ `_PLY2`) (+ `_REFORM`) | the splat(s) as part 1; the reform target (if any) as part 2 |
| *(nothing)* | the **intro production** (`productions/intro/intro.show`); `assets/demo.show` is the alt effect-catalogue |

Examples:

```bash
# Single splat (your own .ply)
MARTIN_PLY=assets/martin.ply cargo +nightly run --release

# Two Martins morph into a dog
MARTIN_PLY=assets/martin-peace.ply MARTIN_PLY2=martin.ply \
MARTIN_REFORM=doggo.ply cargo +nightly run --release

# A glowing title that assembles from particles
MARTIN_TEXT="MARTIN GAUS" cargo +nightly run --release

# A whole show (see "Sequences" below)
MARTIN_PLY=assets/doggo.ply \
MARTIN_SEQ="text:MARTIN GAUS; splat:doggo.ply; text:GREETINGS; text:CODE ANNEJAN" \
cargo +nightly run --release
```

---

## Where files are loaded from

The demo's splats live in **`assets/`** — the default asset root — so splat names
(`MARTIN_PLY2`, `MARTIN_REFORM`, `splat:` parts) resolve there with no extra setup. To
load splats from a **different folder**, point `MARTIN_PLY` at one of them; its **parent
folder becomes the asset root** and the other names resolve beside it:

```bash
MARTIN_PLY=/other/dir/martin.ply   # → asset root = /other/dir
MARTIN_PLY2=martin-peace.ply        # → /other/dir/martin-peace.ply
MARTIN_REFORM=doggo.ply             # → /other/dir/doggo.ply
```

(In a sequence, `MARTIN_PLY` itself need not appear in the parts — it just sets the root.)

> **Export uncompressed / standard PLY** (e.g. from [SuperSplat](https://superspl.at/editor)).
> The loader rejects SuperSplat's *compressed* format (`missing required properties`).

---

## Full env var reference

| Env var | Default | What it does |
|---|---|---|
| `MARTIN_PLY` | `assets/aegg.ply` | Primary splat / asset-folder override — its parent folder becomes the asset root. |
| `MARTIN_PLY2` | — | A second splat, placed beside the first. |
| `MARTIN_REFORM` | — | Morph target: the source splat(s) turn into this one. |
| `MARTIN_GLB` | — | Load a **`KHR_gaussian_splatting` glTF** (`.glb`) splat scene — the standard splat container (e.g. a TRELLIS single-image→3DGS export). **Alone**: a standalone scene view. **Combined with a seq/compose show**: the scene is *set dressing* placed alongside the morphing splats (same camera + bloom; put the `.glb` in the show's asset root, place it with `MARTIN_GLB_POS`). **NB:** glTF-as-*splats*, distinct from `glb:`/`model:` which load glTF as a real PBR *mesh*. |
| `MARTIN_GLB_SCALE` | `1.0` | Scales the loaded `MARTIN_GLB` scene (handy when an export's native units are tiny or huge). |
| `MARTIN_GLB_POS` | `0,0,0` | Places the `MARTIN_GLB` scene (`x,y,z`) — e.g. `2.2,-0.4,-1.5` to park it beside the morph track. |
| `MARTIN_GLB_DIST` | `5.0` | Orbit-camera distance when `MARTIN_GLB` runs **alone** (smaller = tighter framing). |
| `MARTIN_TEXT` | — | Splat-text: this string assembles out of a ball cloud (glowing). |
| `MARTIN_SEQ` | — | A timeline of parts (see [Sequences](#sequences)). Highest precedence. |
| `MARTIN_SHOW` | — | A **unified scene file** (`.show`) — settings + `[seq]` + `[compose]` + a `[camera]` track in one file. Expands into the other `MARTIN_*` vars (which still override it). See [The unified scene file](#the-unified-scene-file-martin_show). |
| `MARTIN_VALIDATE` | — | `=1` **dry-run**: parse the show, print the resolved timeline (part cue times, effects, compose, camera track) and exit — no render. See [Validate a show](#validate-a-show-without-rendering-martin_validate). |
| `MARTIN_TRANSITION` | — | Default arrival transition for every part: `morph`/`swarm`/`ball`/`fade`/`explode`/`implode`/`drop`/`rain`/`funnel`/`shatter`/`condense`/`swirl` (data-only) or `typewriter`/`wipe`/`sparkle`/`slither`/`vortex`/`outline`/`pen-write` (per-particle shader; `outline`/`pen-write` are text-only). A per-part `~name` overrides it. See [Sequences](#sequences). |
| `MARTIN_DEFORM` | — | Scene-wide **persistent deform** field over every part *and* compose object: `wave`/`cloth`/`ripple`/`twist`/`wind`/`turbulence`/`pulse`/`jitter`/`spiral` — runs the whole time a part is held (great on a `wall:`, or to gently wobble a whole splat scene while you fly around it). A per-part `^name` overrides it. See [Persistent deforms](#persistent-deforms-name-keep-a-part-moving-while-its-held). |
| `MARTIN_DEFORM_AMP` | `1.0` | Scales the deform amplitude — **`0.2`–`0.3` ≈ a gentle wobble on a big scene**, `1` = default, higher = wild. |
| `MARTIN_DEFORM_SPEED` | `2.0` | Deform animation rate — `0.6`–`1` = slow/dreamy, higher = faster. |
| `MARTIN_BEAT` | `1.0` | **Beat-reactive visuals** strength (`0` = off). The score's drums drive the look: kick → a scale "thump", snare → a bloom flare, hat → a shimmer, and any active `^deform` swells on the beat. Per-Shot `beat:<scale>` dials it per shot. See [Beat-reactive visuals](#beat-reactive-visuals). |
| `MARTIN_CAM_PUMP` | `0` (off) | Kick-driven **camera lunge** (a transient pull-in on each kick). **Off by default** — the shake is nauseating over a long loop. Opt in with a small value (`0.04` ≈ the old default) for a single punchy clip. |
| `MARTIN_BG` | — | **Fullscreen background shader** behind the splats (the demoscene classic): `plasma` / `tunnel` / `stars` / `warp` / `rings` / `grid` / `kaleido` / `bolt` (or a number). A custom-material quad parented to the camera, opaque at the far plane so the splats blend over it; fed time + beat (kick brightens). The WGSL is `assets/bg.wgsl` — a `mode` uniform switches effects; edit it / add your own (Shadertoy-ish: work in `p` + `bg.time`). |
| `MARTIN_BG_DIM` | `1.0` | Scales the background brightness so foreground content (a logo, glowing text) reads over it — e.g. `0.4` for a punchy effect dialled back to a backdrop. |
| `MARTIN_FLASH` | `0` | Over-bright **bloom flash on each part cut** (0 = off; `~0.6` = punchy). Synced to the music when parts are `@@`-anchored to beats/bars. |
| `MARTIN_SYNTH_WAV` | — | Render the bundled deFEEST synth (Cinder) to a WAV at this path, then exit — for muxing audio onto a recording. See [Music](#music-the-synth). |
| `MARTIN_MUTE` | — | `=1` silences the **live** synth playback (it plays in the window by default; starts with the show, restarts on Space). Doesn't affect recordings — those mux the WAV. |
| `MARTIN_MUSIC` | — | Play a **pre-rendered WAV** instead of streaming the synth live (this is what the bundle ships). Either way the synth is **streamed** — playback + the show start together ~1 s after launch (the producer renders ahead in the background, ≈7× realtime), so `@@` anchors stay sample-locked. `MARTIN_MUSIC` just skips the render. |
| `MARTIN_STREAM_WAV` | — | Debug: render the score via the **streaming** engine to a WAV and exit (cf. `MARTIN_SYNTH_WAV`'s batch render — the two match within ~1 LSB). |
| `MARTIN_SCORE` | built-in | A **score file** (tracker DSL) defining the music — tempo, sections, drum patterns, dynamics. Drives the synth *and* the `@@anchor` section/bar times. See [The score file](#the-score-file). Example: `assets/score.txt`. |
| `MARTIN_SCORE_DUMP` | — | Write the built-in score to this path as an editable score file, then exit — a ready-to-edit starting point (round-trips through `MARTIN_SCORE`). |
| `MARTIN_CAMERAS` | — | A 3DGS/COLMAP `cameras.json` (graphdeco format); parks the camera at a real capture pose (transformed through the same normalize + rotation as the splats). `MARTIN_CAM_INDEX` picks which shot (default 0). *Experimental:* helps cleanly-captured scenes; soft 360° photogrammetry dumps still render abstract (see the scene heads-up above). |
| `MARTIN_BULGE` | `0.9` | Ball-cloud size at a morph's midpoint, in object-radii. `0` = clean "puzzle-box" reorder (no explosion); `~0.9` = a ball roughly the object's size. (In sequences this is the per-part 3rd timing number instead.) |
| `MARTIN_MORPH_COUNT` | `0` (shorthand) / `200000` (`MARTIN_SEQ`) | Gaussian budget every part is resampled to. `0` = the largest part's natural count (~1.15M for the Martins; crisp, ~20 fps). Lower = faster: **250k ≈ 60 fps, 500k ≈ 40 fps.** |
| `MARTIN_PAIR` | — | `=match` switches morph pairing from **index-rank** (Morton Z-order; default) to **nearest same-colour match**. Rank pairing flows beautifully between *similar* shapes (a truck → a train) but pinches *dissimilar* ones (city → city) through a centre **ball** — distant rank-pairs cross at the centroid. `match` reorders each shot so every splat slides to a nearby, similar-colour splat of the previous shot (grass→grass, tower→tower): short moves, a straight ghostly morph, no ball. Also suppresses the beat ball-pulse (below) so the slide stays clean. See [Sequences](#sequences). |
| `MARTIN_PAIR_COLOR` | `0.5` | Colour weight in the `MARTIN_PAIR=match` cost (`distance² + w·colour²`). Higher = pair more by hue (risks longer moves → ball); lower = pair more by position (risks colour-mismatched slides). `0.5` balances both. |
| `MARTIN_YAW` | `1.4` (front) | Seed the orbit **yaw** in **radians** (e.g. `1.57` ≈ head-on). When set, a recording **holds** this yaw instead of swaying — bake a found scene viewpoint. |
| `MARTIN_PITCH` | `0.12` | Seed the orbit **pitch** in **radians** (0 = eye level, `+` looks down). |
| `MARTIN_WAYPOINTS` | `waypoints.json` | File the **M-key camera waypoints** are written to (and read from on startup). Each marker appends the live orbit pose (target/dist/yaw/pitch) so you can author a camera path while flying — see [live controls](#live-keyboard-controls). |
| `MARTIN_FLY` | — | `=<secs>` **flies the camera through the loaded waypoints** instead of free-orbiting. **If every waypoint has a `t`** the path is a *camera track* — played off the show clock (same move live and recorded, `secs` ignored). Otherwise — **Recording:** the path fills each part's on-screen time (longer `hold` = slower flyby), alternating direction; **Live:** `<secs>` = time per leg (default `2`) for a ping-pong preview. Needs ≥2 waypoints in `MARTIN_WAYPOINTS`. |
| `MARTIN_FPS` | off | `=1` logs smoothed FPS / frame-time + timeline clock every ~0.5 s (the **`I`** key toggles it live + logs a snapshot). |
| `MARTIN_RECORD` | — | Directory to dump one PNG per frame into (the whole timeline; used by `record.sh`). **Recording runs fully headless** — no window, camera → an offscreen image (so it works over SSH / on any compositor, and never captures a black background). Works for `MARTIN_COMPOSE` stages too. |
| `MARTIN_PREVIEW_FPS` | 60 | `=<n>` renders the timeline at `n` fps instead of 60 — **far fewer frames** for a fast preview (rendering frames is the slow part, not the mux). `=8` → ~1/8 the frames. Frame `dt` + camera sway scale with it, so timing/motion stay correct; `record.sh` muxes at the same fps so duration + audio sync hold. Use for quick looks; drop it (or set 60) for the final render. |
| `MARTIN_BENCH` | — | `=<frames>` renders that many frames **headless with no PNG output** and logs the render-only fps, then exits — a clean perf probe (disk-I/O-free). |
| `MARTIN_LOADER` / `MARTIN_LOGO` | off | `=1` shows a **loading screen** (black + progress bar; `MARTIN_LOGO=<png OR svg in the asset root>` adds the logo — an `.svg` is rasterized, so it can be the same artwork the opening mesh was extruded from) until the show is built, then **cross-fades** into the opening logo behind it. Set automatically in a bundled build. (Window-only — not captured in recordings.) |
| `MARTIN_SHOT` | — | Capture a single headless screenshot to this path, then exit ~2 s later. |
| `MARTIN_SHOT_AT` | `6.0` | When (seconds) to take the `MARTIN_SHOT`. |
| `MARTIN_SERVE` | — | `=1` (or `=<port>`, default 7878) starts the **live control bridge** — see below. |
| `MARTIN_FULLSCREEN` | off | `=1` starts borderless-fullscreen; toggle live with **F11 / F**. (Ignored while recording — that needs the fixed window.) |
| `MARTIN_LOOP` | off | `=1` keeps a live window up after the show ends (for tuning). By default a live run **exits when the show is done** (Space restarts). |
| `MARTIN_NORMALIZE` | on | Each part is centred on its **centroid** and uniformly scaled (positions *and* gaussian sizes) so the bulk of its content (90th-percentile radius) ≈ 2 units. Using a percentile, not the bounding box, **ignores stray "floater" splats** that would otherwise shrink the scene to a distant dot — so a 200-unit COLMAP scene and a 1-unit TRELLIS object share one "normal" scale. `=0` keeps raw scales. |
| `MARTIN_ZOOM` | `1.0` | Camera closeness multiplier: **`>1` = closer / more zoomed in, `<1` = pull back**. The camera frames the normalized content up close by default; nudge this to taste. |
| `MARTIN_MESH_COUNT` | `60000` | Target gaussian count when surface-sampling a `mesh:` part (distributed by triangle area; ≥1 per triangle). |
| `MARTIN_MESH_SPLAT` | `0.006` | Gaussian splat **in-plane disk size** for a `mesh:` part, as a **fraction of the mesh's largest dimension** (scale-invariant). Each sample is a flat disk aligned to the surface normal. |
| `MARTIN_MESH_THIN` | `0.2` | Mesh disk thickness as a fraction of `MARTIN_MESH_SPLAT` (how flat the surface splats are). |
| `MARTIN_MESH_RGB` | texture / vertex / `0.8,0.85,0.95` | Flat `r,g,b` fallback for a `mesh:` part. Colour priority: the material's **diffuse texture** (sampled at the UV; PNG/JPEG) > vertex colours > material diffuse > this. |
| `MARTIN_ROT` | — | `rx,ry,rz` euler **degrees** applied to the cloud — e.g. stand a COLMAP scene upright for a "normal" POV. Default = the portrait flip (gives scenes their abstract sideways look). Also orients a `glb:` dissolve (mesh + its splats together). |
| `MARTIN_REEL_POS` | `0,0,0` | `x,y,z` translation of the whole **reel** (the morph timeline) off the world origin. The reel normally sits at the origin; this places the morphing subject **relative to `[stage]` props** (which carry their own `@x,y,z`) — e.g. `0,0.6,0` floats a knot⇄galaxy morph above a placed cityscape. Settings key: `reel_pos = x,y,z`. |

---

## Live control bridge

`MARTIN_SERVE=1` (or `=<port>`, default **7878**) boots the show **windowed** but renders the splats
into an **offscreen image** (shown in the window via a 2D blit), so screenshots are window-independent
(no black-on-unfocused, works over SSH). It then serves a **newline-delimited JSON** protocol — one
object per line, one reply per line — so you can drive the camera + clock and grab frames **without
reloading the show** (huge when the capture is hundreds of MB). It's the engine half of "full MCP".

```bash
MARTIN_SERVE=1 MARTIN_SHOW=productions/austin/austin.show cargo +nightly run --release
```

| Command | Effect |
|---|---|
| `{"cmd":"camera","dist":0.6,"yaw":1.3,"pitch":0.18,"pos":[0,0.1,0]}` | nudge the orbit camera (any field optional) |
| `{"cmd":"seek","t":25.0}` · `{"cmd":"pause"}` · `{"cmd":"play"}` · `{"cmd":"step","dt":0.1}` | move / freeze the show clock |
| `{"cmd":"screenshot","path":"/tmp/m.png"}` | write the current frame (lands a moment after the reply — wait briefly) |
| `{"cmd":"dump_camera"}` | a paste-ready `[camera]` line for the current pose + time — **author the track by flying** |
| `{"cmd":"state"}` | current `t`, `paused`, camera |

While serving, the authored `[camera]` track stands down (the bridge owns the camera) and the live
auto-exit is disabled (it runs until you stop it). Example driver (one connection per command):

```python
import socket, json
def cmd(d):
    s = socket.create_connection(('127.0.0.1', 7878)); s.sendall((json.dumps(d)+'\n').encode())
    r = s.makefile().readline(); s.close(); return r
cmd({"cmd":"seek","t":25}); cmd({"cmd":"camera","dist":0.6}); cmd({"cmd":"screenshot","path":"/tmp/m.png"})
```

### Full MCP — `martin --mcp`

`martin --mcp` (or `MARTIN_MCP=1`) runs a **stdio MCP server** (JSON-RPC 2.0) that proxies to a running
bridge, exposing `camera` / `seek` / `pause` / `play` / `step` / `dump_camera` / `state` as native MCP
tools — and `screenshot` returns the PNG **inline as image content**. It's registered in `.mcp.json`,
so an MCP client (e.g. Claude Code) drives the live engine directly. Two steps:

```bash
# 1. start the engine bridge with your show (windowed):
MARTIN_SERVE=1 MARTIN_SHOW=productions/austin/austin.show cargo +nightly run --release
# 2. the MCP client launches `./target/release/martin --mcp`, which connects to the bridge on 7878.
```

Port: `MARTIN_MCP_PORT`, else `MARTIN_SERVE` if numeric, else 7878. The MCP server is pure stdio (no
Bevy), so it stays a clean JSON-RPC channel; build the binary first (`cargo build --release`).

## Live keyboard controls

When running in a window (not recording):

It's a **free-orbit inspection camera** (orbit `yaw`/`pitch` at `dist` around a look-at target):

| Key | Action |
|---|---|
| `←` / `→` | Orbit **yaw** (around the vertical axis) |
| `↑` / `↓` | Orbit **pitch** (look down / up) |
| `W` / `S` | Zoom **in / out** |
| `A` / `D` | Pan the target **left / right** |
| `Q` / `E` | Pan the target **down / up** |
| `M` | **Log a camera waypoint** → the waypoints file (`MARTIN_WAYPOINTS`) |
| `Space` | Restart the show (timeline back to t=0) |
| `F11` / `F` | Toggle borderless fullscreen |

Seed the framed angle with `MARTIN_YAW` / `MARTIN_PITCH` (radians) + `MARTIN_ZOOM`, then orbit
from there — handy for finding and baking a viewpoint. (Single-image splats from TRELLIS have a
**hollow back**, so don't orbit all the way around them; full multi-angle captures orbit freely.)

**Marking a camera path.** Fly to a pose you like and tap **M** — martin appends the live orbit
pose (`target` / `dist` / `yaw` / `pitch`) **plus the current show-time** (`t`, seconds) to the
waypoints file (`MARTIN_WAYPOINTS`, default `waypoints.json`) and logs the marker. Keep flying and
dropping markers to capture a whole path. The file is plain JSON — an array of poses — and is **read
back on startup**, so M *continues* an existing path across runs.

**Camera as a track.** Because **M** stamps the show-time, a path you author this way is a
**music-timed camera track**: every marker has a `t`, so the camera is played *straight off the show
clock* — the move hits each pose at the same musical moment live and in the recording, with no
part-window heuristic. Hand-edit the `t` values to retime a move to the beat. (Drop the `t` keys
from every marker and it falls back to the part-filling flyby below — a hand-written legacy path.)

**Flying the path back.** With ≥2 markers, set **`MARTIN_FLY=<secs>`** and the camera flies the
path instead of free-orbiting (smoothstep easing through each marker, shortest-way yaw):

```bash
# preview the path live — ping-pongs there-and-back, ~<secs> between each marker
MARTIN_PLY=assets/train.ply MARTIN_FLY=2 cargo +nightly run --release

# bake it into a recording — the path fills the clip; a longer hold = a slower flyby
MARTIN_PLY=assets/train.ply MARTIN_SEQ="splat:train.ply @18,1" MARTIN_FLY=2 ./record.sh train_flyby.mp4
```

**Recording fills each *part's* on-screen window with one pass of the path**, **alternating
direction** — part 0 flies first marker → last, part 1 last → first, and so on. Two upshots: the
camera is *always moving* — it reaches the turn-marker exactly as the morph begins, so there's **no
dead pause before the transition** — and its position is *continuous* across the morph (the next
subject picks up from that marker and reverses — no jump). **A part's flyby lasts as long as its
`hold`** — want it slower, hold longer. Live, `<secs>` is the time per waypoint leg (default `2`)
and it ping-pongs the path on a loop, for judging the shape. Waypoints pin exact camera poses, so
replay them on `.ply`s that share a frame.

**Same flyby on two subjects, with a morph between** — when two splats normalize to the same spot
(the train + truck from one dataset do), one path frames both:

```bash
# train flies the path forward, then (continuous, no jump) the truck flies it back through the
# morph. Each subject's flyby lasts its hold (~14–15 s here). Bulge 0 = a straight morph.
MARTIN_PLY=assets/train.ply \
MARTIN_SEQ="splat:train.ply @13,1 ~fade; splat:truck.ply @12,3,0 ~morph" \
MARTIN_MORPH_COUNT=1000000 MARTIN_FLY=2 ./record.sh train_truck_flyby.mp4
```

(`@hold,morph,bulge` in seconds — the camera flies each subject for its full on-screen time, so
just set the holds to taste; no need to match a "pass length" — the flyby stretches to fit.)

> **Heads-up on raw scene `.ply`s.** A bare splat from a 360° capture (no camera poses) carries
> lots of under-constrained background "needle" splats and only blends coherently along its
> *capture trajectory* — so it may look streaky from an arbitrary orbit no matter the distance.
> Either **crop it to the subject** in SuperSplat (see `ART-DIRECTION.md`), or use a `.ply` whose
> capture cameras are available. Clean objects (TRELLIS, cropped captures) orbit fine.

---

## Sequences

`MARTIN_SEQ` is the composable mode: a list of **parts** that morph into one another,
each transition flowing through a ball cloud. It's either a `;`-separated string **or a
path to a file** with one part per line (`#` starts a comment, blank lines are skipped).

**Part grammar:**

```
text:STRING                      # splat-text (glowing)
wall:LINE1|LINE2|LINE3           # a multi-line WALL of text (| = newline), or wall:greets.txt
image:logo.png                   # a PNG in the asset folder, rasterized to gaussians (a logo)
svg:logo.svg                     # an SVG, rasterized (vector→pixels, MARTIN_SVG_PX wide) → gaussians
mesh:logo.dae                    # a 3D mesh (.dae/.obj/.stl/.ply), surface-sampled into gaussians
glb:badge.glb                    # a real glTF mesh: rendered crisp, THEN dissolves into its own
                                 #   sampled splats (coincident by construction) which morph on
shader:warp                      # a fullscreen WGSL effect as an INTERLUDE: the splats clear and
                                 #   the effect plays full-frame (warp/plasma/tunnel/stars/rings/grid/kaleido/bolt), fading
                                 #   in/out across the part — a demoscene effect between scenes
splat:name.ply                   # a splat (filename in the asset folder)
splat:a.ply+b.ply                # several splats, auto-arranged side by side
…any of the above… @hold,morph,bulge   ~entrance   ^deform[:amp]   exit:departure   rot:rx,ry,rz   flock:N   @@anchor   backdrop:name   raster:mode   flash:strength
```

> **Per-Shot look overrides (scene-scoped looks):** `flash:<strength>` flares the cut-bloom on *this*
> Shot's entry (overrides the global `MARTIN_FLASH` — punch one drop, not every cut); `^deform:<amp>`
> scales this Shot's wobble (e.g. `^wave:0.4` gentle, `^twist:2` violent) on top of `MARTIN_DEFORM_AMP`;
> `beat:<scale>` dials this Shot's beat-bounce (`beat:0` = still through the drop, `beat:1.6` = punchier)
> so the kick reaction rides only on *some* Shots, not the whole show. Together with the existing
> per-Shot `backdrop:` / `raster:`, a Shot fully controls its own look.

> **Domain vocabulary** (see [`DOMAIN.md`](DOMAIN.md)): the section is `[reel]` and a line is a **Shot**;
> the modifiers above are the canonical spellings. Older spellings still parse as aliases:
> `[seq]`→`[reel]`, `~transition`→`~entrance` (the `~` slot), `out:`→`exit:`, `cluster:`→`flock:`,
> `bg:`→`backdrop:`, and the setting `morph_count`→`budget`. New shows should use the canonical words.

### `[scenes]` — write the show as an arc of scenes

Instead of a flat `[reel]`, you can author the **Showbook arc directly**: a `[scenes]` block groups
Shots under named **Scenes**, and each Scene's look is inherited by its Shots. It flattens to the exact
`[reel]` the engine runs — pure sugar, content-agnostic (a Shot is any `splat:`/`mesh:`/`wall:`/
`image:`/`svg:`/`glb:`/`shader:` line). Example ships at `assets/examples/arc.show`:

```
[scenes]
scene opener  @@intro  backdrop:off                 # a scene opens a beat + sets its look
  glb:defeest.glb  @8,3  ~morph  exit:explode
scene party   @@drop   backdrop:plasma  ^wave        # the whole scene waves on a plasma backdrop
  splat:galaxy.ply  @5,2  ~morph
  splat:knot.ply    @5,2  ~morph  backdrop:bolt      # a Shot's own backdrop overrides the scene's
  text:HELLO        @4,2  ~typewriter                # inherits ^wave + plasma
```

On flatten: the Scene's `@@anchor` stamps its **first** Shot (the rest flow after it); the Scene's
`backdrop:`/`^deform` apply to every Shot that doesn't set its own. (`[arc]` is an alias of `[scenes]`.
If a show has *both* `[scenes]` and an explicit `[reel]`, the `[reel]` wins.) A future revision may add
per-Scene camera moves and density — see [`DOMAIN.md`](DOMAIN.md) §5/§9.

**Per-part raster mode** (`raster:<mode>`): the fork's debug-shading views, colour each gaussian by a
channel instead of its RGB. `color` (default, normal render) · `depth` · `normal` · `position`
(colour by XYZ → a rainbow gradient) · `classification` · `flow` (optical-flow) · `velocity`. Set
the whole show's default with **`MARTIN_RASTER=<mode>`**; a part's `raster:` token overrides it.
`depth`/`normal` need geometry to read (great on captures/`mesh:`/`glb:`, flat on a synthetic cloud);
`position` looks great on anything — e.g. `text:deFEEST ~outline raster:position` = outline-revealed
letters in a position-colour rainbow.

**Per-part background** (`bg:<name>`): switches the fullscreen background shader **from that part
on** (sticky until the next `bg:` token): `plasma` / `tunnel` / `stars` / `warp` / `rings` / `grid` /
`kaleido` / `bolt`, or **`bg:off`** for pure black. This makes the background a second energy curve
across the show — e.g. `bg:off` for the intro, `bg:bolt` on the drop, back to `bg:stars` for the
outro. `MARTIN_BG` (if set) is the default before the first token; `MARTIN_BG_DIM` stays global.

`image:`/`svg:` parts share the crispness knobs **`MARTIN_IMG_STRIDE`** (pixel subsample, default
`2`) and **`MARTIN_IMG_SPLAT`** (gaussian size, `0.012`); an `svg:` also takes **`MARTIN_SVG_PX`**
(the width it rasterizes the vector to before sampling, default `512` — raise it for a crisper logo).

The optional trailing `@hold,morph,bulge` sets, in **seconds** (and ball amount):
- **hold** — how long to rest on this part once it arrives (default `1.5`)
- **morph** — how long the morph *into* this part takes (default `3.0`)
- **bulge** — ball-pulse explosiveness, `0`–`~1.4` (default `0.9`; **`morph` transition only**)

The optional trailing **`~transition`** picks *how* the part arrives (the ball is just one
of them). It can sit anywhere on the line, but reads best last:

**Data-only** (a built source cloud the morph flies in from):

| `~name` | How it arrives |
|---|---|
| `~morph` | flows from the **previous** part's shape (Morton-paired), with the ball-pulse `bulge` — the default after part 0 |
| `~swarm` | like `~morph`, but each particle takes a curled, **flocking/swarming** detour between the two scenes (the `@_,_,N` timing value tunes the swarm strength) |
| `~ball` | assembles out of a fuzzy ball shell — the default for part 0 |
| `~fade` | fades up on the spot (opacity 0 → in) |
| `~explode` | gathers in from an outward burst |
| `~implode` | expands out from a dense point |
| `~drop` | falls straight down into place |
| `~rain` | falls in from scattered high points — a staggered shower (vs `~drop`'s straight fall) |
| `~funnel` (`~pour`) | pours in from a tall narrow column above, fanning out + down |
| `~shatter` (`~shards`) | re-assembles from ~8 tumbling shards (a shattered object flying back together) |
| `~condense` (`~fog`/`~haze`) | condenses out of a wide faded haze — positions converge + opacity fades up |
| `~swirl` | sweeps/spirals in around the vertical axis (cheap, straight-line) |
| `~extrude` (`~rise`, `~pop`) | rises out of a flat silhouette into 3D — a logo extruding from its 2D shape into its mesh (best head-on, or on a deep object) |
| `~helix` (`~dna`, `~spiral`) | reels in off a tall spinning column — a DNA / barber-pole assemble |
| `~fold` (`~unfold`) | unfolds sideways out of a vertical seam, like opening a folded sheet |
| `~zoom` (`~telescope`, `~warp-in`) | rushes in from far — a telescope / hyperspace zoom into place |

**Per-particle** (the fork shader staggers each splat — great for text):

| `~name` | How it arrives |
|---|---|
| `~typewriter` (`~type`) | reveals left→right like a typewriter |
| `~wipe` | a hard slab edge sweeping across the x axis |
| `~sparkle` | random per-particle twinkle-in (HDR bloom makes it flash) |
| `~slither` | staggered lateral wobble that settles into place |
| `~vortex` | spins/unwinds into place (continuous, shader-driven) |
| `~outline` | **text only** — traces the *filled* font's letter outlines in pen order (a glowing neon draw-on) |
| `~pen-write` (`~pen`) | **text only** — real handwriting: traces a *single-stroke* font's centerline in pen order |

`MARTIN_TRANSITION=<name>` sets a default for **every** part (handy for trying one out); an
explicit per-part `~name` wins over it.

**Departures** (`out:name`) — where `~name` says how a part *arrives*, `out:name` says how it
**leaves**: it morphs to a faded "gone" cloud as a distinct step at the end of its hold (before the
next part arrives), so the object dissolves away instead of cross-morphing straight to the next. The
following part then assembles fresh (a Morph/Swarm arrival after a departure becomes a ball).

| `out:name` | How it leaves |
|---|---|
| `out:wash` | flows off sideways and fades — washed away |
| `out:disperse` (`out:dust`) | scatters outward in all directions and fades — blown to dust |
| `out:evaporate` (`out:rise`) | drifts upward and fades — rises away |
| `out:sink` (`out:fall`) | falls straight down and fades — drops out the bottom |
| `out:explode` (`out:burst`) | flung ballistically outward from the centre and fades — a burst (punchier than `disperse`'s wash) |

Example: `splat:dog.ply out:wash ; text:HELLO ~rain out:disperse ; splat:dog.ply ~ball` → the dog
washes away → text rains in then blows to dust → the dog re-forms from the void.

**Per-part rotation** (`rot:rx,ry,rz`, euler **degrees**) orients a single part, *baked into its
shape* — so different parts can sit at different angles in one show (and the morph between them
reorients smoothly), without a global `MARTIN_ROT` that would tilt everything. It composes on top of
`MARTIN_ROT`. Handy when one object's natural frame differs — e.g. a flat logo that needs standing
up while the dog stays upright: `glb:defeest.glb rot:-90,0,0 ; splat:dog.ply ; text:HELLO`. (Works
for `glb:` too — the mesh and its sampled splats rotate together.)

**Cluster** (`cluster:N`) replicates a part into **N scattered, randomly-rotated copies** — a
"serving" rather than a lone object (e.g. `mesh:bitterbal.obj cluster:9` → a plate of 9 bitterballen,
each tumbled differently). Deterministic (frame-stable for recording); the whole pile is sized to
frame as one. Great for snacks, confetti, a swarm of logos, …

---

## Meshes + splats in one universe

Real triangle meshes and gaussian splats **coexist** — the splat crate draws splats in the
`Transparent3d` phase with a depth test against the opaque pass, so a mesh occludes splats behind
it and splats blend over it. Three ways to combine them, from simplest to richest:

| Source | Where | What it does |
|---|---|---|
| `mesh:foo.dae` | seq + compose | Surface-**samples** a mesh (`.dae`/`.obj`/`.stl`/`.ply`) into gaussians — it *becomes* splats and morphs like anything else. No real mesh is drawn. |
| `model:foo.glb` | **compose only** | Renders a **real** PBR glTF mesh as a rigid prop *alongside* the splat objects (shares camera + depth). It doesn't morph; it spins/bobs/dissolves-by-scale via the compose `in`/`out` timing. |
| `glb:foo.glb` | **seq only** | The **dissolve**: renders the real mesh crisp AND samples its splats from that *same loaded mesh*, so they coincide. Choreographs splats↔mesh↔splats (see below). |

**The `glb:` dissolve choreography** (one part — `glb:badge.glb`):

1. the part assembles **as splats** (its `~ball`/`~morph`/… — mesh hidden),
2. the splats **materialize** into the crisp solid mesh (`MODEL_FADE`, ~0.6 s),
3. it **holds** crisp (splats suppressed → no poke-through; the mesh is truly crisp + readable),
4. it **dissolves** back to splats as its OWN step (`DISSOLVE_LEN`, ~1.2 s, carved from the end of
   the hold so it finishes *before* the next part's morph),
5. those splats **morph on** to the next part.

So `glb:badge.glb → splat:dog.ply → text:HELLO` reads as: random → badge splats → **mesh** → badge
splats → dog → text. `MARTIN_ROT` orients the mesh **and** its splats together (they always coincide).
`MARTIN_MESH_COUNT`/`_SPLAT`/`_THIN` tune the sampled disks.

**Why `glb:` and not just `model:` + `mesh:` of the same board?** Two separately-exported files can't
be aligned by a rotation: the mesh *sampler* reflects Y (Y-down convention) while the *renderer*
rotates, and a `.dae` and a `.glb` of the same object have different native frames — so a sampled
`mesh:` and a rendered `model:` of "the same" board are mirrored/rotated apart and impossible to line
up by eye. `glb:` sidesteps it entirely by sampling the gaussians **from the rendered mesh itself**
(normalizing the gaussians and placing the mesh on the identical `(centroid, scale)`), so they're
coincident *by construction* — no rotation/scale/mirror knobs.

### Authoring / editing the mesh assets (Blender)

Edit the 3D objects (the deFEEST logo, the Ægg board, …) in **Blender 5.0** (`/usr/bin/blender-5.0`
— installed, all-AMD friendly). **glTF is the canonical format** both ways: Blender imports + exports
it natively (materials/PBR included) and martin loads `.glb` directly, so there's no lossy hop.

Workflow:
1. **Import** the object into Blender — `File ▸ Import ▸ glTF 2.0` on `assets/<name>.glb`. (The old
   `.dae` is legacy; if you only have a `.dae`, convert once with `assimp export in.dae out.glb`.)
2. **Save it as `<name>.blend`** — that becomes your editable master (keep it out of git; it's big).
3. Edit / finish the model.
4. **Export** `File ▸ Export ▸ glTF 2.0 (.glb)` over `assets/<name>.glb`. That's the file martin uses
   (`glb:<name>.glb` for the dissolve, or `model:<name>.glb` as a compose prop). **So: edit the
   `.blend`, ship the `.glb`.**

Tips so it reads well once sampled into splats:
- **Give it real depth/volume.** The sampler scatters disks over the *surface*, so a paper-thin logo
  becomes a flat disc — a little extrude/bevel gives the splat cloud body.
- **Set each part's material Base Color.** The `glb:` sampler reads `StandardMaterial.base_color`
  (one flat colour per material/primitive) — separate materials (e.g. yellow ring / blue field /
  text) → correctly coloured splats. (Textures aren't sampled by `glb:` yet — use materials.)
- **Apply transforms** (`Object ▸ Apply ▸ All Transforms`) so the export is in a clean frame; martin
  orients the whole thing with `MARTIN_ROT` (it moves mesh + splats together).
- Poly count isn't critical — sampling is area-weighted to the splat budget (`MARTIN_MESH_COUNT`).

**Other handy ways to edit** (besides Blender):
- **Inkscape → Blender (the logo route).** A logo edits best as **2D vector**: tweak the
  ellipse/text/colours in Inkscape, save SVG, then Blender `Import ▸ SVG` → curves → extrude →
  export `.glb`. Far nicer for crisp text/shapes than pushing polygons.
- **`gltf-transform` via `npx`** (no install) for quick non-interactive edits to a `.glb`:
  `npx @gltf-transform/cli inspect assets/foo.glb` (what's inside), `… recenter`, recolour, optimise.
- **Edit `.gltf` as text.** Export glTF *Separate* (`.gltf`) and hand-edit **material colours, names,
  node transforms** in the JSON (geometry is binary, but those bits are plain JSON — git-diffable).
- **Headless Blender Python** — `blender-5.0 --background --python edit.py` for reproducible scripted
  edits (recolour, apply transforms, re-export) — good for a repeatable asset bake.
- **SVG → OpenSCAD extrude (the deFEEST logo, no Blender on the box).**
  `pipeline/svg_extrude_logo.py` builds the 3D logo from the official vector `assets/defeest.svg`: it
  splits the SVG into its three colour layers (yellow base ellipse / blue ellipse / yellow letters)
  and `openscad`-extrudes each, centred on z=0 (so the logo is **mirror-symmetric** — reads front
  *and* back) in a coin/badge layout: the yellow **rim** (ellipse minus the blue field) and the
  **letters** stand proud at the same thickness, the **blue** field is thinner so it sits **inset**,
  and only the rim's outer edge gets a very subtle soft bevel (the letters stay crisp). One command
  regenerates both `assets/defeest.glb` (canonical
  glTF) and `assets/defeest.dae` (what the show loads via `mesh:`), each layer its own coloured
  material. Self-verify headless: `pipeline/logo-check.compose` (one `mesh:` line) +
  `MARTIN_SHOT=/tmp/x.png MARTIN_SHOT_AT=3`.
- **`pipeline/svg_import.py` — generic SVG → 3D asset import.** The reusable version of the above for
  *any* flat SVG: `pipeline/svg_import.py logo.svg` → `assets/logo.glb` + `.dae`. It groups the SVG's
  filled paths **by fill colour** (inline, `fill=`, or CSS-class `.st0{fill:…}` — one material each) and
  extrudes each, stepped by paint order so the
  foreground colour sits proudest (centred on z=0 → mirror-symmetric, no cap-plane mush). `--depth`
  tunes thickness, `--uniform` makes every colour equal, `--clean` runs an Inkscape `object-to-path`
  normalise first. This is the **ahead-of-time** route (bake the mesh once, then `mesh:`/`glb:` it);
  the **runtime** counterpart is `image:logo.png`, which rasterises a flat PNG to splats live. Only
  *filled* paths import (no strokes/gradients/bitmaps); same-colour regions merge into one layer (to
  split figure/ground that share a colour — as the deFEEST logo does — use `svg_extrude_logo.py`).
**Asset provenance** (licences declared in `REUSE.toml`):
- **deFEEST logo** — official vector `defeest.svg` (via [Iconape](https://iconape.com)); the 3D
  `defeest.dae`/`.glb` are extruded from it (`pipeline/svg_extrude_logo.py`). `defeest-logo.png` from
  [defeest.nl](https://defeest.nl). deFEEST = Anne Jan; **MIT**.
- **Bornhack Ægg / badge board** (`aegg.dae`/`.glb`, `bornhack2026-hardware.dae`) — from
  [codeberg.org/Ranzbak/bornhack2026-hardware](https://codeberg.org/Ranzbak/bornhack2026-hardware),
  **MIT © Badge.Team**.
- **`bitterbal.obj`** (+ `bitterbal.glb`, derived via `pipeline/bitterbal_glb.py`) — © [Maali](https://maali.nl),
  used **with Maali's permission** (`LICENSES/LicenseRef-Maali.txt`).
- **BornHack logo** — the host camp's wordmark, from the
  [bornhack-website](https://github.com/bornhack/bornhack-website) repo (**BSD-3-Clause © 2016–2018 BornHack**);
  `bornhack.glb`/`.dae` are extruded from `bornhack.svg` (`pipeline/svg_import.py`).
- `bawl-e.dae` — [bawlsec.com](https://bawlsec.com/)'s logo (Anne Jan's object via scene.rs/deFEEST,
  originally from [M42D](https://bawlsec.com/authors/m42d.html)) — stays **local** (gitignored, not
  published). The large splat bakes (`*.ply`) stay out of git too — regenerate from the meshes.

- **Flat-logo shortcut:** if it's really 2D, skip the mesh — `image:logo.png` rasterises a PNG
  straight to crisp flat splats (edit the PNG in GIMP/Inkscape). The 3D `.glb` is only for the
  mesh-dissolve.

> **`~outline` vs `~pen-write` (both text-only).** Same shader mechanism (reveal along the pen
> path), different font. `~outline` traces the bundled *filled* font (DejaVu) → a glowing neon
> outline drawing itself on. `~pen-write` traces a bundled *single-stroke* font (Relief
> SingleLine CAD, OFL) via `ttf-parser`, keeping each stroke **open** (not closed back into a
> loop) and respecting pen-lifts between strokes → genuine centerline handwriting, upper- and
> lowercase. Tune stroke weight with `MARTIN_PW_SPLAT` (default `0.006`) / `MARTIN_PW_STEP`.

(The first part has nothing to morph *from*, so `~morph` there falls back to `~ball`.)

### Persistent deforms: `^name` (keep a part moving while it's held)

A `~transition` plays **once** on arrival. A trailing **`^deform`** instead runs the *whole* time
the part is on screen — a living wobble, perfect on a `wall:` of text (orbit it with the free
camera to catch the ripple in 3D):

| `^name` | What it does |
|---|---|
| `^wave` (`^flag`) | a sine ripple travelling across the wall — a flag in wind |
| `^cloth` (`^billow`) | 2D undulation (x and y out of phase) — a hanging-cloth billow |
| `^ripple` | concentric waves from the centre outward — a drop in water |
| `^twist` (`^curl`) | the wall slowly curls and uncurls — a 3D banner roll |
| `^wind` (`^gust`) | a gusting sideways sway (particles lag by position) + flutter — blown in the wind |
| `^turbulence` (`^turb`, `^churn`) | a churning 3D force field — the shape swirls/boils in place |
| `^pulse` (`^breathe`) | the whole shape breathes in and out about its centre |
| `^jitter` (`^shake`) | a fast per-particle shake — nervous, glitchy energy |
| `^spiral` (`^pinwheel`) | a radial pinwheel — the shape swirls/curls about the vertical axis |

```
wall:GREETINGS|TO ALL|DEMOSCENERS   @8,1   ~fade   ^wave
```

`MARTIN_DEFORM=<name>` is a **scene-wide field**: it sets a default deform for every seq part *and*
every placed compose object — `MARTIN_DEFORM=wind` blows the whole stage at once. An explicit
per-part / per-object `^name` always wins over the field.
The deform is independent of the arrival transition, so a part can `~fade` in *and* `^wave`. (Off
by default → no movement; it's a default-off branch in the fork shader, see the fork's `CHANGES.md §5`.)
**`MARTIN_DEFORM_AMP`** scales the wobble (`0.2`–`0.3` ≈ gentle on a whole scene) and
**`MARTIN_DEFORM_SPEED`** its rate — so you can load a scene and softly wobble it while you fly
around it:

```bash
MARTIN_PLY=assets/train.ply MARTIN_DEFORM=wave MARTIN_DEFORM_AMP=0.3 MARTIN_DEFORM_SPEED=1 \
  MARTIN_ZOOM=1.5 cargo +nightly run --release
```

### Beat-reactive visuals

With a score playing (always, unless muted), the drums drive the look — `MARTIN_BEAT=<scale>` tunes
it (`0` = off, `2`–`3` = exaggerated):

| hit | reaction |
|---|---|
| **kick** | a quick scale **thump** on the cloud/objects + a camera **pump** (lunges in) |
| **snare** | a **bloom flare** (over-bright pulse) |
| **hat** | a fine bloom **shimmer** |
| kick+snare | swells any active **`^deform`** so a `^wave`/`^ripple` part *pumps* with the track |

It's the same data path as the synth + `@@anchor`s, so the visuals react to *any* score. Camera
pump + thump are deterministic (clock-driven), so they bake identically into a recording.

### Cue-anchoring: `@@anchor` (lock a part to the music)

By default parts are laid **end-to-end** (each starts when the previous finishes). A trailing
**`@@anchor`** token instead pins a part's *start* to an absolute time on the **music clock**
(the ported deFEEST score — 140 BPM; see [Music](#music-the-synth) below), so the show locks to
the track no matter how you retime the parts before it:

| `@@anchor` | Part starts at… |
|---|---|
| `@@drop` | a **section** boundary — `intro` / `build` / `drop` / `breakdown` / `climax` / `outro` |
| `@@bar32` (or `@@bar:32`) | **bar 32** (`32 × BAR`) |
| `@@beat:64` | **beat 64** (`64 × BEAT`) |
| `@@12.5` | **12.5 seconds** (raw) |

The part still uses its `@morph` to arrive and holds until the next part starts. Anchors should
increase down the show. Example — the title holds until the drop, then the dog hits *on* it:

```
text:MARTIN GAUS              @2,3
splat:doggo.ply  ~morph  @@drop @1,3,1.0     # morphs in exactly when the drop lands
image:defeest-logo.png ~ball  @@outro
```

`MARTIN_FLASH=<strength>` adds an over-bright **bloom flash on each part cut** (0 = off,
default; `~0.6` is punchy) — the demoscene scene-cut snap, synced to the music when parts are
anchored to beats/bars.

**Inline example — a full show:**

```bash
MARTIN_PLY=assets/doggo.ply \
MARTIN_SEQ="text:MARTIN GAUS @2,2.5,0; splat:doggo.ply @2,3,0.9; text:GREETINGS @1.5,2.5,0.9; text:DEFEEST CINDER @1.5,2.5,0.7; text:CODE ANNEJAN @2,2.5,0.6" \
cargo +nightly run --release
```

**File example** — put this in a file (say `my-show.seq`):

```
# Martin Gaus — title → two faces → dog → greetings → credits
text:MARTIN GAUS @2.5,3,0
splat:martin.ply+martin-peace.ply @2,3,0.6   # the two Martins, side by side
splat:doggo.ply @2,3.5,0.9                    # …become the dog
text:GREETINGS @1.5,2.5,0.9
text:CODE ANNEJAN @2.5,3,0.6
```

…and run it:

```bash
MARTIN_PLY=assets/doggo.ply MARTIN_SEQ=~/my-show.seq cargo +nightly run --release
```

A ready-to-run example ships at **`assets/examples/director.show`** (`MARTIN_SHOW=assets/examples/director.show`)
— a full director sequence with every part `@@`-anchored to the score's sections
(intro→build→drop→…→outro), so the visuals ride the music.

All parts are resampled to one gaussian count (`MARTIN_MORPH_COUNT`, default 200k in
sequences) and the camera is framed once over everything, so it never pops between parts.

---

## Composition — the stage (`MARTIN_COMPOSE`)

Where `MARTIN_SEQ` is a *timeline* (one object morphs into the next), `MARTIN_COMPOSE=<file>` is a
**stage**: many objects on screen **at once**, each placed + animated, the camera flowing around
them. The shipped, ready-to-run example is **`assets/examples/stage.show`**:

```bash
MARTIN_SHOW=assets/examples/stage.show cargo +nightly run --release
```

(`MARTIN_COMPOSE=<file>` loads a bare compose body directly — the same lines that go under a `.show`'s
`[compose]` section.)

Each line is one object: a `<source>` (any of `text:` / `wall:` / `image:` / `mesh:` / `splat:`)
followed by placement + motion tokens:

```
splat:doggo.ply                @0,0,0      *1.1  spin 0,18,0
splat:martin.ply               @-1.9,.3,0  *.7   spin 0,-22,0   in build
text:deFEEST                   @0,1.7,0    *.8   bob .12        in drop
mesh:bornhack2026-hardware.dae @0,-1.6,.3  *.7   spin 20,40,0   in climax
```

| token | meaning |
|---|---|
| `@x,y,z` | position on the stage |
| `*s` | scale |
| `rot a,b,c` | static orientation (euler degrees) |
| `spin a,b,c` | auto-rotation, **degrees/sec** |
| `bob amp` | vertical bob amplitude |
| `drift dx,dy,dz` | translation velocity, units/sec |
| `sway a,b,c` | oscillating rotation amplitude (deg) — swings front-on, so a hollow-back splat never shows its empty side |
| `in <anchor>` / `out <anchor>` | fade in / out at an `@@`-style time (section / `bar:N` / `beat:N` / seconds) |
| `~transition` | each object **assembles in** via its own arrival (`~ball`/`~rain`/`~funnel`/`~shatter`/…) instead of a plain fade — the same vocabulary as the morph timeline. **`text:~pen-write` handwrites the letters in** (single-stroke font, traced stroke-by-stroke — same as the reel). |
| `^deform[:amp]` | a persistent **wobble** while it's up (`^wave`/`^wind`/…); the optional `:amp` scales its strength (`^turbulence:0.3`) |
| `tint:<mode>` | recolour the sampled splats with a CPU colour routine: **`fry`** (deep-fried bitterbal — beige crevices → brown crust peaks, noise-driven), **`rainbow`** (clean left→right spectrum), **`brand`** (deFEEST blue→gold gradient). |

So objects **fade in on the music** (the stage builds with the track), spin/bob/drift in place, and
the camera slowly **auto-orbits** the whole arrangement (grab it any time with the arrow keys).
`MARTIN_MORPH_COUNT` caps splats **per object**.

**Stage + timeline together (tracks).** A compose stage can run *alongside* the morph timeline: set
`MARTIN_COMPOSE` **and** an explicit `MARTIN_SEQ` (or `MARTIN_PLY`/`MARTIN_TEXT`). The morph timeline
is then the **hero** track (it frames the camera) and the compose objects are placed around it —
objects, text and meshes living in one scene. (Compose *alone*, with no morph track requested, still
frames + auto-orbits itself.) Place the objects near the origin (`@±1`) so they sit in the hero's
frame. Example:
```bash
MARTIN_PLY=assets/doggo.ply \
MARTIN_SEQ="text:HELLO ~ball; splat:doggo.ply ~swarm" \
MARTIN_COMPOSE="mesh:bitterbal.obj @-1.3,.4,0 *.5 spin 0,40,0; text:deFEEST @1.3,.5,0 *.5 sway 0,25,0" \
cargo +nightly run --release
```

## The unified scene file (`MARTIN_SHOW`)

Once a show grows past a one-liner, gather it into a single **`.show` file** instead of juggling a
pile of `MARTIN_*` vars plus a `waypoints.json`:

```bash
MARTIN_SHOW=assets/example.show cargo +nightly run --release
MARTIN_SHOW=assets/example.show ./record.sh example.mp4
```

**Galleries** in [`assets/examples/`](assets/examples/) show the palette (text-only, no `.ply` needed):
`transitions.show` (every `~arrival`), `shaders.show` (every fullscreen effect), `deforms.show`
(every `^deform`). Run any with `MARTIN_SHOW=assets/examples/<name>.show …`, or read one on paper
with `MARTIN_VALIDATE=1`.

A `.show` has four kinds of section — see [`assets/example.show`](assets/example.show):

| section | what it is |
|---|---|
| *(top, before any header)* `key = value` | **settings** — each becomes `MARTIN_<KEY>` (`morph_count = 180000` → `MARTIN_MORPH_COUNT`, `deform = wind`, `bg = plasma`, …) |
| `[seq]` | the **hero** morph timeline — verbatim [`.seq`](#sequences) syntax |
| `[compose]` | the **stage** of placed objects — verbatim [`.compose`](#composition--the-stage-martin_compose) syntax |
| `[camera]` | a music-timed **[camera track](#live-keyboard-controls)** — order-free `t=<s> pos=x,y,z dist= yaw= pitch=` lines. `t` is seconds **or `@@anchor`** (`t=@@drop` locks the keyframe to a music section, like a seq part) |

It's deliberately pure sugar: the file **expands into the env** (the settings become `MARTIN_*`, the
`[seq]`/`[compose]` bodies become `MARTIN_SEQ`/`MARTIN_COMPOSE`), so everything above works exactly
the same — and **an explicit env var on the command line still wins** over a setting in the file
(`MARTIN_DEFORM=turbulence MARTIN_SHOW=x.show …` overrides just that one knob). The only part that
isn't an env var, the inline `[camera]` track, is handed straight to the camera (and auto-enables
`MARTIN_FLY`). Author the camera live — fly around and tap **M** at musical moments — then paste the
logged poses into the `[camera]` section, or hand-edit the `t`'s to lock moves to the beat. A
keyframe time can also be a **`@@anchor`** (`t=@@drop`) so the move snaps to a music section even if
you retune the tempo.

### Validate a show without rendering (`MARTIN_VALIDATE`)

**`MARTIN_VALIDATE=1`** parses the whole show and prints the resolved timeline — then exits, no
window, no render. Each seq part with its cue start time + transition/deform/departure, the compose
stage, and the camera track. A fast "is my show right, and what does it look like on paper?" check
(typos in the show also print `unknown …` lines on stderr):

```bash
MARTIN_VALIDATE=1 MARTIN_SHOW=my.show cargo +nightly run --release
```

## Music (the synth)

martin carries a procedural synth + a **section/beat music clock**, ported (MIT) from Cinder's
(Kristian Vlaardingerbroek, deFEEST) `term-demo` — `src/audio.rs` + `src/score.rs`. The clock
is 140 BPM with a six-section arc (`intro → build → drop → breakdown → climax → outro`), an
Am–F–C–G chord progression driving the bass + stab, and a melodic **lead** — all editable in the
[score file](#the-score-file). Those section/bar/beat times are what `@@anchor` (above) pins parts to, so the visuals lock to the
track. It **plays live in the window** so you can tune the score by ear — the synth is **streamed**
(rendered in time-ordered segments on a background thread), so playback + the show start together
about a second after launch and the producer races ahead; a brief loader covers the lead-in. It
restarts on **Space** (`MARTIN_MUTE=1` skips the music, `MARTIN_MUSIC=<wav>` plays a pre-rendered
track). For **recording**, live playback is skipped and the synth instead renders **offline to a
WAV** (the batch engine) that ffmpeg muxes onto the frames (sample-accurate):

```bash
# 1. render the synth to a WAV (renders, then exits — no window)
MARTIN_SYNTH_WAV=/tmp/track.wav cargo +nightly run --release

# 2. record the (anchored) show to PNG frames
MARTIN_PLY=$PWD/assets/doggo.ply MARTIN_SEQ="…@@drop…@@outro…" MARTIN_RECORD=/tmp/frames \
  BEVY_ASSET_ROOT=$PWD cargo +nightly run --release

# 3. mux: video + audio (+ a fade to match the synth's own fade-out)
ffmpeg -framerate 60 -i /tmp/frames/frame_%05d.png -i /tmp/track.wav \
  -vf "fade=t=out:st=206:d=2.6" -c:v libx264 -pix_fmt yuv420p -c:a aac -shortest out.mp4   # st ≈ clip end
```

The built-in track is **~3:30** (209 s); anchor the final part near `@@outro` so the recording
covers the whole track. (The length is the score's total bars — edit `assets/score.txt` to change it.)

---

## The score file

The music is **data**, not code. By default it's the built-in score, but `MARTIN_SCORE=<file>`
loads a **tracker-DSL** score, and `MARTIN_SCORE_DUMP=<file>` writes the built-in out as an editable
starting point (it round-trips — the dumped file renders byte-identical music). **One score file
drives both** the synth and the `@@anchor` section/bar times, so retiming the music retimes the
show. The shipped example is **`assets/score.txt`**.

```
bpm 140

# chord progression, one per bar, cycling (a note + optional `m` = minor): drives bass + stab
chords Am F C G

# section <name> <bars> <phase-bars,csv> [fill]
section intro      8             # 8 bars, one phase, no fill
section build     20  9,10 fill  # 20 bars = phase0 (9 bars) + phase1 (10) + 1 fill bar
section drop      28  13,14 fill

# <section>.<kick|snare|hat|stab>  p<N>|fill:  16 steps   (x = hit, . = rest; spaces ignored)
build.kick  p0:   x... .... .... x...
build.kick  p1:   x... ..x. .... x...
build.snare p1:   .... x... .... x...
build.kick  fill: x... .... .... x...

# <section>.lead  p<N>|fill:  16 note slots — the MELODY (note names like A4 / C#5 / Eb3; . = rest)
drop.lead   p0:   A5 . E5 .  . C5 . .  D5 . E5 .  . A5 . .
drop.lead   p1:   E5 . . A5  . G5 . E5  . . D5 .  C5 . . .

# <section>.arp  p<N>|fill:  same note grammar — a SECOND melodic line (a sparkly counter-melody)
climax.arp  p0:   A6 E6 C6 E6  A6 E6 G6 E6  A6 E6 C6 E6  D6 E6 G6 A6

# dynamics 0..1 per section — `v` constant, or `a>b` to ramp across the section (a riser!)
gain  intro 0.5  build 0.85  drop 1  breakdown 0.6  climax 1  outro 0.7
sub   intro 0.25 build 0.25>0.8  drop 1  breakdown 0.15  climax 0.9  outro 0.4   # build's sub rises into the drop
mids  intro 0.5  build 0.7  drop 0.9  breakdown 0.6  climax 1  outro 0.45
```

- **`chords`** is one root per bar (cycling), e.g. `Am F C G`; a trailing `m` = minor. It moves the
  **bass** (root) and **stab** (triad). **`lead`** is the melody and **`arp`** is a second melodic
  line — both note-lanes: 16 whitespace-separated note tokens per slot (`A4`, `C#5`, `Eb3`, or
  `.`/`-` for a rest), written per phase like the drums.
- **16 steps per bar** (16th notes); patterns you don't write are silent.
- A section's `<bars>` is its total length; `<phase-bars>` is how the kit pattern changes *within*
  it (plus a trailing fill bar when `fill`). Section **names** are what `@@anchor` matches — so a
  custom score with custom section names re-anchors the show to them.
- The *instrument* (how a kick/stab actually sounds — the synth DSP) stays in code; the file is the
  **composition**. **`assets/score.txt` is loaded by default** — just edit it and run (no recompile,
  no env var; the visuals re-anchor to the new section times). `MARTIN_SCORE=<other>` overrides it,
  and the file is also embedded in the binary as the fallback, so the music isn't duplicated in code.

> **Bundling.** Because the score *and* `MARTIN_SEQ` are plain files, a single-binary build bakes
> them (+ the splats) into the executable — see [Packaging & release](#packaging--release).

## Recording to video

`record.sh` (in the repo root) builds the demo, renders frames, **renders the synth and muxes it
in** (so the `.mp4` has the music — honours `MARTIN_SCORE`, skipped by `MARTIN_MUTE`), and fades the
video out with the track. It inherits all the `MARTIN_*` env vars:

```bash
# from the repo root
MARTIN_PLY=assets/doggo.ply \
MARTIN_SEQ="text:MARTIN GAUS; splat:doggo.ply; text:CODE ANNEJAN" \
./record.sh my_show.mp4
```

The clip length is computed automatically from the parts' `@hold,morph` (and `@@anchor`) timings.

**Flagship example — the whole story in assets** (`assets/examples/truck-show.show`): the truck rides
the music while wavy neon titles draw on, the camera flies the marked waypoint path, and it morphs
into the deFEEST logo mesh for the outro (score + camera path are referenced from the `.show` itself):

```bash
MARTIN_SHOW=assets/examples/truck-show.show ./record.sh truck_show.mp4
```

To grab a single still instead:

```bash
MARTIN_TEXT="MARTIN GAUS" MARTIN_SHOT=/tmp/title.png MARTIN_SHOT_AT=6 \
cargo +nightly run --release
```

---

## Packaging & release

Ship a whole show as **one self-contained binary** — assets baked in, no external files, no env:

```bash
./pipeline/release.sh            # → a portable single binary that plays the baked-in show
```

`cargo build --release --features bundle` *is* the pipeline: `build.rs` reads **`bundle.toml`** (the
show — a `seq`/`compose` + `score`/`logo`/`morph_count`), auto-collects every `.ply`/PNG it
references, lz4-compresses them into the executable, and bakes the show string in. At startup the
binary self-extracts to a temp dir and plays the show (loader screen while it decompresses); env
vars still override for debugging. Fonts + the default score are already compiled in, so only splats
(+ a logo PNG) ship. CI bakes the **intro** production (`productions/intro/bundle.toml` → `intro.show`);
the root `bundle.toml` defaults to `assets/demo.show`. Both use only *light* assets (procedural demo
shapes + a few tracked meshes + text), so a bundle lands around ~180 MB.

**Portability.** A binary linked on a bleeding-edge distro (openSUSE Tumbleweed, glibc 2.43) fails on
older ones (`GLIBC_2.xx not found`). `release.sh` links against an **old glibc** via
[`cargo-zigbuild`](https://github.com/rust-cross/cargo-zigbuild) when `zig` + `cargo-zigbuild` are on
`PATH` (`TARGET_GLIBC`, default 2.31 → Ubuntu 20.04+/Debian 11+/Mint 20+) — built on the dev box,
runs everywhere; the GPU/audio/window libs are dlopen'd and present on any desktop. One-time setup:
`cargo install cargo-zigbuild` + put `zig` on `PATH`. Native fallback (+ a warning) otherwise.

The procedural demo shapes (`sphere`/`cube`/`torus`/`helix`/`galaxy`/`star`/`wave`/`ring`/`knot`/
`mobius`/`supershape`) are synthesized by **`build.rs`** (`build/gen_splats.rs`) — any one a show
references is generated on build if its `.ply` is missing, so no separate step is needed.
Mesh → "proper" splat (offline Blender→Brush bake, all-AMD): `pipeline/mesh-splat.sh`.
Export a score to a standard MIDI file (to share an arrangement with a DAW / notation tool):
`python3 pipeline/score_to_midi.py [assets/score.txt] [out.mid]` — lead / sax / bass / chord tracks.

## Performance notes (Radeon 860M iGPU, Vulkan)

It's fill-rate bound and the depth sort scales with gaussian count:

| `MARTIN_MORPH_COUNT` | Frame rate |
|---|---|
| `250000` | locked 60 fps |
| `500000` | ~40 fps |
| `0` (max, ~1.15M) | ~20 fps — crisp, best for offline video / a beefier machine |

Use the lower counts for a smooth **live** demo and `0` for the final **rendered** video.
Run `--release`: the debug build is for fast iteration only.
