# SHOWBOOK — Op de Camping (deFEEST)

> Het storyboard is het **bron-artefact** van deze productie: eerst hier ontwerpen (minuten), dan pas
> renderen (tientallen minuten). Dit boek is ook de capture-boodschappenlijst voor op de camping.
> Engine-werk hoort niet hier — als een scène iets van de engine vraagt dat nog niet kan, komt het in
> *Engine-vragen* onderaan en blijft de scène gewoon beschreven zoals bedoeld.

**Muziek:** Op de Camping (Ome Henk-flavoured DnB, Gm, 140 BPM, ~228s) — `productions/camping/score.txt`
(deze productie heeft z'n eigen arrangement; de engine-builtin is neutraal tropical-house).
**Secties:** intro 0 · build 14 · drop 43 · breakdown 99 · climax 129 · outro 185 · einde 228.
**Show-file:** `camping.show` (full-fat, lokale captures; de CI/bundle-etalage is `productions/intro/`).

## De arc (besloten 2026-06-13)

deFEEST drijft in → het deFEEST-dorp bouwt op de camping → het feest barst los → **zonder deFEEST
is het kut** (nacht, triest) → **met deFEEST = het ultieme genot** (peak party) → greetings & credits.
Emotionele ruggengraat: de zonder-deFEEST-dip (breakdown) maakt de met-deFEEST-climax groter.
Merk + camping-props zijn echte captures; de blobs/splats ⇄ mesh dissolves zijn de magie ertussen.

Spelregel per scène: **één idee per scène, en per scène veranderen de régels** (dichtheid,
camera-regime, look) — niet alleen de content. Niets staat ooit stil in beeld; alles wat verschijnt
heeft een entree én een aftocht.

## De zes scènes

| # | sectie (tijd) | werktitel | wat zie je | camera | status |
|---|---|---|---|---|---|
| 1 | intro (0–14) | **deFEEST drijft in** | Pikdonker. De 3D deFEEST-mesh drijft in beeld, draait zacht, dissolved dan tot blobs/splats (`glb:defeest.glb` mesh⇄splat). Het merk opent, wordt deeltjes. | langzaam indrijven, zacht om de mesh | ◪ uitgewerkt |
| 2 | build (14–43) | **deFEEST-dorp op de camping** | De deFEEST-blobs hervormen tot camping-scenery: het deFEEST-dorp op de camp, props die indrijven (bier/krat, club-mate, BBQ, tent). Bouwt op met de muziek. **Vereist echte captures.** | orbit → langzame push vooruit | ◪ uitgewerkt (captures nodig) |
| 3 | drop (43–99) | **Het feest barst los** | De hele camping leeft: deFEEST-splats die overal **golven** (`^wave`), happy **regenboog-backdrop-shader**, meer props — tafel vol eten, deFEEST-leden die juichen. Snelle ball-pulse morphs op de beat. **Vereist captures.** | snel maar frontaal, push-ins op de hits | ◪ uitgewerkt (captures nodig) |
| 4 | breakdown (99–129) | **Zonder deFEEST** | Nacht. `bg:stars` sterrenveld, gedimd. Trieste tekst over hoe het leven kut is zónder deFEEST (ZONDER deFEEST / IS HET KUT). Camp leegt weg. **Geen captures (tekst + bg).** | zakt, drift traag weg | ◪ uitgewerkt |
| 5 | climax (129–185) | **Met deFEEST** | Peak party: meer juichende deFEEST-leden, regenbogen, grote "explosies" (= dikke ball-pulse morph-bursts, de sexy soort, géén scatter), bitterballen-regen (`bitterbal.glb`, bestaat). MET deFEEST = het ultieme genot. **Captures (leden) nodig.** | energiek maar frontaal, push-ins op de hits | ◪ uitgewerkt (captures nodig) |
| 6 | outro (185–228) | **Greetings & credits** | Greetings naar andere scene- + camping-groepen (scroll/walls) — i.h.b. de hosts **BornHack** (`svg:bornhack.svg`) — dan de credits (crew: ANUS\|KLOOT\|CINDER), deFEEST-signature (pen-write); dissolve → zwart. **Geen captures (tekst).** Greet-namen nog invullen. | frontaal, trekt langzaam terug | ◪ uitgewerkt |

*Status-trap: □ idee → ◪ uitgewerkt (shots + timing hieronder) → ▣ gebouwd (in camping.show) → ★ goedgekeurd.*

### Scène-uitwerkingen

*(per scène die ◪ wordt: shots met seconden, welke parts/transities, welke captures, wat de camera
exact doet. Nog leeg — dit is de tekentafel.)*

## Capture-boodschappenlijst (op de camping te filmen)

**Props — als crops, één object per capture.** Organisch/rommelig spul splat prachtig; strak
menselijk maaksel (caravan, krat) is de zwakke plek van gaussians — overweeg daarvoor de
mesh-route (`model:`/`glb:`) of accepteer de zachtheid.

| capture | rol in de show | notities | status |
|---|---|---|---|
| krat bier (40 cm) | **schaal-ijk** voor álle captures + prop (scène 2) | COLMAP-schaal is willekeurig: het krat is de meetlat | □ |
| club-mate fles | prop (scène 2) | herkenbaar silhouet; etiket leest leuk in splats | □ |
| campingstoel | prop, breakdown/climax | open vouwstoel leest beter dan dichte | □ |
| tent(je) | hero-prop, build/climax | doek = goed splatbaar; stokken worden zacht | □ |
| gasstel/BBQ | prop, scène 2 + bron van vuurvliegjes | met rooster/pannetje voor herkenbaarheid | □ |
| de camping-hoek | **hero-omgeving** (backdrop) | alleen vanuit de gefilmde kijkhoek gebruiken | □ |
| tafel vol eten | prop (scène 3) | volle tafel = rommelig = splat prachtig | □ |
| deFEEST-leden juichend | scène 3 + climax | bewegend = lastig; juichen = stil poseren, armen omhoog | □ |

**Capture-regels** (uitgebreider in `ART-DIRECTION.md`):
1. **Eén licht-sessie.** Splats bakken hun belichting in — alle props onder hetzelfde licht
   (één avond/bewolkt blok), anders vloekt de compositie.
2. **Normalisatie-conventie** (in SuperSplat, vóór gebruik): voet op y=0, voorkant naar −X,
   schaal geijkt op het krat, opgeslagen als `captures/<naam>.ply`.
3. **Crop hard.** Alles wat geen onderwerp is gaat eruit; de omgeving komt van de hero-capture.
4. Omgeving: filmen vanuit de hoek waaruit de demo 'm toont; vrije viewpoints bestaan niet.

## Engine-vragen die uit dit boek volgen

*(pas bouwen als een ◪-scène ze echt nodig heeft)*

- **Scene-scoped looks** — per scène eigen `bg`/`bg_dim`/`flash`/`deform` (nu globaal). → scène 1/4 vs 3/5.
- **Dichtheids-dramaturgie** — van ~200 deeltjes (vuurvliegjes) naar 160k en terug. → scène 1/6.
- **Camera-regimes** — een echte flythrough (pad + kijk-langs-pad) naast orbit. → scène 2/5.
- **Harde cuts** — instant part-wissel op de beat (morph ≈ 0 + flash). → scène 3.
- **Beat-reactie-variatie** — nu bounced álles altijd op de kick (MARTIN_BEAT is globaal); per scène
  moet de reactie kunnen wisselen (thump / flare / shimmer / niets). → alle scènes.
- **Happy regenboog-backdrop** — bg.wgsl heeft plasma/kaleido (regenboog-achtig) maar geen expliciete
  vrolijke "rainbow" mode; evt. een nieuwe bg-mode toevoegen. → scène 3.
- **Cluster van crispe meshes** — `cluster:N` kloont nu alleen ge-samplede *splats* (`mesh:`), niet de
  crispe `glb:` dissolve-mesh; en `model:` rendert niet in een `[seq]` (alleen `MARTIN_COMPOSE`). Een
  echt *bord met 9 scherpe bitterballen* vraagt: cluster:N op de `glb:` dissolve (N scherpe meshes →
  N splat-clusters). Tot dan = zachte splat-blobs. → scène 3/5 (bitterballen-regen).

## Logboek

- 2026-06-13 — productie-structuur opgezet (`productions/camping/`), arc + scène-raamwerk + boodschappenlijst neergezet. Trellis-als-permanent-decor verworpen: alles op het scherm heeft een levenscyclus nodig.
- 2026-06-13 — BornHack-logo (host-camp) toegevoegd: `assets/bornhack.{svg,glb,dae}` (witte fill,
  via `svg_import.py`). Eerst op twee plekken gezet (build + outro); daarna **uitgedund** — beide
  logo's tonen nu één keer: deFEEST in de intro (`glb:defeest.glb`), BornHack in de outro-greeting
  (`svg:bornhack.svg`), z'n narratieve thuis. Bitterballen blijven splat-blobs (`mesh:…cluster:N`):
  crispe-mesh-cluster is geen engine-feature (zie *Engine-vragen*).
- 2026-06-13 — alle 6 scènes □→◪ besloten (samen, na het beluisteren van de track). Scènes 1/4/6
  bouwbaar zónder captures (mesh-open / nacht-tekst / greetings+credits); 2/3/5 vereisen echte
  captures (camp-props, eten-tafel, juichende leden). Captures-lijst aangevuld (club-mate, eten-tafel,
  leden). Engine-vraag toegevoegd: happy regenboog-bg-mode.
