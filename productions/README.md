# productions/ — one folder per demo

martin is a **show engine**; a demo is a **production**. The engine stays theme-agnostic (see the
docs rule in `CONTRIBUTING.md`); everything theme-specific lives in a production folder:

```
productions/<name>/
├── SHOWBOOK.md     # the storyboard — the SOURCE artifact: scenes, beats, capture shopping list
├── <name>.show     # the show file(s) the engine plays (MARTIN_SHOW=productions/<name>/<name>.show)
├── bundle.toml     # this production's single-binary recipe (MARTIN_BUNDLE=productions/<name>/bundle.toml)
├── score.txt       # the production's music (tracker DSL) — once it has its own
└── captures/       # the production's real splat captures (large .ply files, gitignored)
```

The split test for any new work: *is this an engine feature or production content?* Scene blocks,
density ramps, camera regimes → engine (`src/`). The fireflies-from-the-BBQ idea → the production's
showbook. Reusable, tested building blocks → `parts/`.

Each production declares its **`kind`** in its `.show` settings — **`kind = intro`** (light,
always-buildable, repo assets + procedural splats only — the bundleable showcase) or **`kind = demo`**
(full-fat, may lean on big *local* captures that don't ship). See [`DOMAIN.md`](../DOMAIN.md) for the
domain model behind `kind`, `[reel]`/`[scenes]`, and the Shot vocabulary.

Current productions:

- **intro** (`kind = intro`) — the small always-buildable engine showcase that CI bakes into the
  downloadable bundle (repo assets + procedural splats only, its own simple track).
- **camping** (`kind = demo`) — Op de Camping, the main demo, in design (see its SHOWBOOK).
- **austin** / **nyc** (`kind = demo`) — thin productions: each is just a `.show` recipe that flies a
  **local aerial photogrammetry capture** (downtown Austin / Manhattan, Google Aerial View → COLMAP →
  Brush, ~1.3M splats, SH3). Authored in the `[scenes]` arc layer; the city is the hero, beat-reactive.
  The `.ply` is **gitignored Google Maps Content — not shippable** (see
  [`pipeline/AERIAL-CITIES.md`](../pipeline/AERIAL-CITIES.md)); only the `.show` recipe is committed, so
  others rebuild the capture themselves with their own API key. Any displayed frame needs the
  **"Imagery ©Google"** attribution (each show carries it in the outro).
- **cities** (`kind = demo`) — the smooth multi-city **morph tour**: one city disperses to a fuzzy
  sphere and reassembles as the next (the deFEEST ball-pulse signature applied to skylines). The morph
  *is* the transition. All four captures (austin/nyc/chicago/seattle) are baked; `cities.show` /
  `cities-easy.show` fly all four, `cities-energy.show` is a tighter wide-grazing-drone cut over just
  the three strongest captures (chicago's flatter capture collapses to top-down at drone angles). Same
  gitignored-capture / ©Google constraints as above. Also here: **cities-defeest** (the shareable causal
  loop — city tour balls into a bitterbal → the deFEEST logo → back) and **city-bite**.
- **synthwave** (`kind = demo`) — a bold neon space-journey on **martin's own composed music** (no
  copyright concerns — every asset is procedural or repo-tracked): a galaxy → Saturn → rocket → ufo →
  supershape → helix → torus → ufo arc via `splat:`/`splatgen`, an explosive drop entrance, a fullscreen
  blinder into the climax, neon backdrops + sparks.
- **born2defeest** (`kind = demo`) — the **original hardstyle BornHack 2026 demo** (~220 s, 145 BPM): a
  full 6-scene `[reel]` arc (deFEEST logo → host-camp build + ægg hatch → party drop → "Wij zijn deFEEST"
  breakdown → climax → credits/fade) with its own `score.txt` and full `[camera]`/`[sync]`/`[caption]`
  tracks. Exercises nearly every engine feature (`glb:`/`svg:`/`splat:`/`mesh: flock:`/`text:`/`wall:`,
  all `~entrances` + `^deforms` + per-shot `backdrop:`).
- **parade** (`kind = demo`) — character-parade: the whole deFEEST cast struts past as ONE colour-matched
  morphing cloud (doggo→martin→LUIGI→train→truck→peace-sign) → balls into a bitterbal → resolves to the logo.
- **bitterbal** (`kind = demo`) — bitterbal showcases: **zwerm** (26-ball lissajous swarm), **wereld**
  ("de wereld is een bitterbal"), **cosmic-snack**, **lissajous-galaxy**, **alles** (the big multi-scene
  reel+stage demo).
- **guinea** (`kind = demo`) — a short dark-humour bit: the deFEEST logo floats in, morphs into a live
  guinea pig (**cavia**) on a plate, which morphs into the cooked Andean dish (**cuy**, served splayed),
  over a cheerful fairground (kermis) polka. A `[reel]` of `mesh:`-sampled textured glTF morphs +
  a `[stage]` plate; exercises `ground:`/`disk:`/`pair=match` + the sRGB/bilinear mesh-colour fixes.
- **credits** / **defeest** — the credits roll (`[compose]` layout) and the deFEEST title-build.

> **Composing a scene visually?** Pose a production's `[stage]`/`[compose]` props (and camera) in
> **Blender** and round-trip them back into the `.show` with `pipeline/blender_bridge.py` — see the
> "Blender ↔ martin bridge" section in [`AGENTS.md`](../AGENTS.md). (Programmatic `[reel]`/`path:`/
> `travel:` content isn't poseable; load one reel frame as a `backdrop=` to pose stage props over it.)

Future candidates: kantoor, supermarkt, koffieshop, …

The engine's built-in default score (`assets/score.txt`, `include_str!`'d) is theme-AGNOSTIC — a
generic tropical-house groove — so the default demo + example shows don't ship a theme. Each
production owns its own score (e.g. `productions/camping/score.txt`, the Op de Camping arrangement;
`productions/intro/score.txt`, the 124-BPM intro cut) and points its `.show`/`bundle.toml` at it.
