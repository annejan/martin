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
  *is* the transition. Currently **austin ⇄ nyc**; chicago/seattle drop into the `[reel]` once their
  splats are baked. Same gitignored-capture / ©Google constraints as above.

Future candidates: kantoor, supermarkt, koffieshop, …

The engine's built-in default score (`assets/score.txt`, `include_str!`'d) is theme-AGNOSTIC — a
generic tropical-house groove — so the default demo + example shows don't ship a theme. Each
production owns its own score (e.g. `productions/camping/score.txt`, the Op de Camping arrangement;
`productions/intro/score.txt`, the 124-BPM intro cut) and points its `.show`/`bundle.toml` at it.
