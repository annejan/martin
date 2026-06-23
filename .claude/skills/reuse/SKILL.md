---
name: reuse
description: >-
  Make `reuse lint` green again — find unlicensed files, classify their licence, annotate them in
  REUSE.toml (path globs) + add any missing LICENSES/<id>.txt. Invoke whenever reuse lint fails, before
  a push, or after adding new assets/files (a recurring chore: every new file needs SPDX info).
---

# Fixing REUSE compliance

`reuse lint` must stay green (CI gates on it). Every file needs SPDX copyright + licence info, via a
header OR a `REUSE.toml` glob.

## 1. Find what's missing
```
reuse lint 2>&1 | grep -A40 "no copyright and licensing information:"
```
Note: reuse lints the working tree incl. **untracked** files (but skips `.gitignore`d ones, e.g.
`.claude/`). New untracked assets are the usual culprit.

## 2. Classify each file's real licence — do NOT guess wrong on third-party content
- **martin's own** (src, our `.show`/`.ply`/docs/scripts): MIT, © Anne Jan Brouwer.
- **Third-party assets**: find the REAL terms — check the file's embedded metadata (`strings x.otf | grep -i copyright`, SVG `<dc:rights>`), a bundled LICENSE/README in the dir, or the source repo. Match the existing pattern in REUSE.toml (e.g. fonts → OFL/GPL, BornHack community art → `LicenseRef-BornHack`, Maali → `LicenseRef-Maali`). If genuinely unknown, flag it to the user rather than inventing a licence.

## 3. Annotate in REUSE.toml
Add a `[[annotations]]` block with a path glob (prefer globs over per-file), a comment explaining
provenance, the copyright holder, and the SPDX id:
```toml
[[annotations]]
path = "assets/newthing/*.otf"
SPDX-FileCopyrightText = "2014 Some Foundry"
SPDX-License-Identifier = "GPL-2.0-only"
```
Use `precedence = "override"` when a narrower rule must beat a broader `src/**`-style one.

## 4. Add the LICENSE text if the SPDX id is new
reuse needs `LICENSES/<SPDX-id>.txt` to exist for every id used. If a dep bundles the licence text,
copy it: `cp "dir/LICENSE.txt" LICENSES/GPL-2.0-only.txt`. Otherwise `reuse download <id>` (needs net).

## 5. Verify
```
reuse lint 2>&1 | grep -iE "Congratulations|not compliant|Files with"
```
Goal: `Files with copyright/licensing information: N / N` + "Congratulations". Then `git add` the
files + REUSE.toml + any new LICENSES/ text and commit.

## Notes
- This is part of the pre-push gate set: `cargo fmt --all --check`, `cargo clippy --all-targets -D warnings`, `cargo test --release`, `reuse lint`.
- Root markdown docs + many config files are already covered by globs in REUSE.toml — add new docs to the appropriate list there.
