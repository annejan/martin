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

Current productions: **camping** (Op de Camping — the main demo, in design, see its SHOWBOOK) and
**intro** (the small always-buildable engine showcase that CI bakes into the downloadable bundle —
repo assets + procedural splats only, its own simple track). Future candidates: kantoor, supermarkt,
koffieshop, …

The engine's built-in default score (`assets/score.txt`, `include_str!`'d) is theme-AGNOSTIC — a
generic tropical-house groove — so the default demo + example shows don't ship a theme. Each
production owns its own score (e.g. `productions/camping/score.txt`, the Op de Camping arrangement;
`productions/intro/score.txt`, the 124-BPM intro cut) and points its `.show`/`bundle.toml` at it.
