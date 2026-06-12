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

Current productions: **camping** (Op de Camping — in design, see its SHOWBOOK). Future candidates:
kantoor, supermarkt, koffieshop, …

Known theme-leak, deliberate for now: the *engine* still embeds the camping score as its built-in
default (`assets/score.txt` via `include_str!`) because the default demo plays it. It moves here
when a second production brings its own music.
