# SHOWBOOK — Op de Camping (deFEEST)

> Het storyboard is het **bron-artefact** van deze productie: eerst hier ontwerpen (minuten), dan pas
> renderen (tientallen minuten). Dit boek is ook de capture-boodschappenlijst voor op de camping.
> Engine-werk hoort niet hier — als een scène iets van de engine vraagt dat nog niet kan, komt het in
> *Engine-vragen* onderaan en blijft de scène gewoon beschreven zoals bedoeld.

**Muziek:** Op de Camping (Ome Henk-flavoured DnB, Gm, 125 BPM) — `assets/score.txt` (engine-embedded;
verhuist naar deze map zodra er een tweede productie met eigen muziek is).
**Secties:** intro 0 · build 14 · drop 43 · breakdown 99 · climax 129 · outro 185 · einde 228.
**Show-file:** `camping.show` (full-fat, lokale captures) · `camping-ci.show` (repo-only, voor de bundle).

## De arc: werkelijkheid ↔ magie

Eén verhaal in plaats van losse objecten. **Zonder deFEEST** is de camping een doods stilleven;
**met deFEEST** komt ze tot leven. Visueel: de show begint en eindigt *fotorealistisch* (echte
captures, rustige camera) en wordt in het midden *magisch* (de werkelijkheid ontbindt in deeltjes,
danst, en komt weer samen). De reis werkelijkheid → magie → werkelijkheid volgt de muziek exact.

Spelregel per scène: **één idee per scène, en per scène veranderen de régels** (dichtheid,
camera-regime, look) — niet alleen de content. Niets staat ooit stil in beeld; alles wat verschijnt
heeft een entree én een aftocht.

## De zes scènes

| # | sectie (tijd) | werktitel | wat zie je | camera | status |
|---|---|---|---|---|---|
| 1 | intro (0–14) | **Vuurvliegjes** | Pikdonker. Een handvol lichtpuntjes drijft in, zwermt, condenseert tot het deFEEST-logo. | bijna statisch, heel licht drijvend | □ idee |
| 2 | build (14–43) | **De tent op** | Typografie bouwt op: OP DE CAMPING wordt een muur/tunnel; sterren faden in; eerste capture-glimp. | van orbit naar langzame push vooruit | □ idee |
| 3 | drop (43–99) | **Het feest barst los** | De muur explodeert in de captures (Ægg, dog, …): snelle morphs op 4-bar-grenzen, zwerm-erupties, flash op de hits. | snel, zwiepend, push-ins op de beat | □ idee |
| 4 | breakdown (99–129) | **Zonder deFEEST** | Alles loopt leeg: de camping als bevroren, grijs stilleven; deeltjes regenen weg; sterren dimmen. ZONDER deFEEST / IS HET KUT. | zakt, drift traag weg | □ idee |
| 5 | climax (129–185) | **Met deFEEST** | Voor het eerst alles tegelijk op het podium (compose): captures + tekst + zwermen, camera vliegt ertussendoor. | flythrough tussen de objecten | □ idee |
| 6 | outro (185–228) | **De deeltjes gaan slapen** | Credits (pen-write), logo; de wolk verstrooit terug naar vuurvliegjes → zwart. Symmetrie met scène 1. | trekt langzaam terug | □ idee |

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
| krat bier (40 cm) | **schaal-ijk** voor álle captures + prop | COLMAP-schaal is willekeurig: het krat is de meetlat | □ |
| campingstoel | prop, breakdown/climax | open vouwstoel leest beter dan dichte | □ |
| tent(je) | hero-prop, build/climax | doek = goed splatbaar; stokken worden zacht | □ |
| gasstel/BBQ | prop + bron van vuurvliegjes (scène 1!) | met rooster/pannetje voor herkenbaarheid | □ |
| de camping-hoek | **hero-omgeving** (backdrop) | alleen vanuit de gefilmde kijkhoek gebruiken | □ |
| mensen/de crew | climax | bewegend = lastig; stilstaand poseren | □ |

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

## Logboek

- 2026-06-13 — productie-structuur opgezet (`productions/camping/`), arc + scène-raamwerk + boodschappenlijst neergezet. Trellis-als-permanent-decor verworpen: alles op het scherm heeft een levenscyclus nodig.
