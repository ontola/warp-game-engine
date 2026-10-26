# Ondoorzichtige naaldkronen en de afstand tot Nanite Foliage (r51)

Status: **geïmplementeerd achter `SIDEWAYS_TREE_CROWN=opaque`, standaard uit,
niet gemeten op een echte GPU.** De standaard blijft `cards`. Dit is een
meetbare proef, geen promotie en geen Nanite-implementatie.

## Vergelijking met UE5.7 Nanite Foliage

Bronnen: Epics [Nanite Foliage-documentatie](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage)
en aantekeningen bij de Unreal Fest-talk *Large Scale Animated Foliage in The
Witcher 4*. De demo draait op een gewone PS5 met 60 FPS: ongeveer 500.000
bomen, 1,7 miljoen struiken en 7 miljoen grasinstanties, met ongeveer 4 ms
Nanite-budget voor het bosdeel (schaduwen apart begroot).

| Onderdeel | UE5.7 / Witcher 4 | Warp vóór r51 | Gat |
|---|---|---|---|
| Bladeren | Volledig gemodelleerde, ondoorzichtige geometrie, 1 tot 10 miljoen triangles per boom. Epic mat kaarten 3 tot 4 keer trager dan geometrie (in hun software-rasterizer) | Naaldkaarten met alpha test, 1.088 kroontriangles per boom | Groot |
| Geheugen | Nanite Assemblies: tot 65.000 geïnstantieerde onderdelen per boom, gedeelde clusters (3,5 GB naar 29 MB op schijf) | Drie gebakken meshes | Groot |
| Verte | Per cluster naar voxels (4x4x4-bricks) zodra dat minder fout geeft dan triangles; normaalverdeling per voxel | Alpha-triangles tot ongeveer 150 per boom; hele-boomvolumes afgewezen | Middel tot groot |
| Doorlopen | Hiërarchisch, met streaming | Vlakke lus per owner over alle residente clusters | Klein voor de huidige bomen (63 tot 87 clusters, zie [r46](cluster-traversal-r46.md)), groot zodra bomen echte geometrie krijgen |
| Wind | Starre skinning per bot, geïnstantieerde animatievarianten, animatie uit onder een schermgrootte | Eén afschuiving per boom op elke afstand | Middel |
| Zonneschaduw | Virtual shadow maps, gecachete pagina's, verre bomen in rustpositie | 4096² + 2048² PCF, daarna een ellipsoïde kroonveld | Groot; de proef met gepagineerde naaldschaduw is al een VSM-light |
| Belichting | Lumen, two-sided foliage, TSR herkent dunne geometrie | Vaste backlight, sky-SH, heuristische bounce | Middel |
| Inhoud | 28 soorten, struiken, meerdere biomen | Drie sparren, geen loofbomen of struiken | Groot |

Wat al in lijn is: GPU-gestuurde visibility buffer, één windtoestand voor alle
views en schaduwen, en de procedurele weide met ondoorzichtige sprieten.

Volgorde van werk, in de praktijk bijgesteld: de top-down clusterdoorloop is
in r46 al geprobeerd en levert voor de huidige bomen niets op, omdat elke boom
maar 63 tot 87 clusters heeft. De boomkosten komen uit het aantal bomen maal
de overdraw van kaarten. Daarom begint r51 met de geometrie zelf.

## Wat r51 doet

Elke naaldtak (frond) van de drie sparren beeldt de hele textuur
`authored_pine_albedo.png` af op een gebogen kaart van acht triangles; elke
boom heeft er 136. Dat is precies het geval waar Epic assemblies voor
gebruikt: één onderdeel, vele keren geplaatst.

1. `games/sideways/tools/twig_part.py` bakt het onderdeel offline: het
   alphasilhouet van de textuur, gesloten op 384 px met drempel 0,3 op de
   gemiddelde alpha, vereenvoudigd met 1 px tolerantie en constrained-Delaunay
   getrianguleerd. Resultaat: `assets/meshes/pine_twig_part.kmesh`, 1.937
   triangles in UV-ruimte (silhouet-IoU 0,911 tegen het masker zelf).
2. `tree_assets::opaque_fronds` plaatst bij het laden één kopie per frond:
   elke onderdeelvertex krijgt positie en normaal barycentrisch uit de
   kaarttriangle die zijn UV bevat (buiten de kaart: het vlak van de
   dichtstbijzijnde). UV's blijven die van het onderdeel, dus de kleur komt uit
   dezelfde textuur. Daarna volgt de bestaande taper; triangles worden na de
   taper opnieuw naar hun normaal gewonden.
3. Materiaal: `OPAQUE_NEEDLES | TWO_SIDED` in plaats van
   `ALPHA_TEST | TWO_SIDED`. De nieuwe vlag (1024) krijgt in `shade.wgsl`
   exact de naaldbelichting van de kaarten (tint per boom, backlight, AO 0,85,
   f0 0,01); de raster leest geen alpha. Canopy-veld en boomtuning herkennen
   de vlag als kroon.
4. Vereenvoudiging: `TexturedGeometry` in plaats van `AlphaCards`.

Per boom: 1.088 naar 263.432 kroontriangles (ongeveer 240 keer), 63 tot 87
naar ongeveer 4.640 clusters en 6 naar 14 LOD-niveaus. De grofste LOD blijft
131 tot 172 triangles. Het bouwen van de drie boommeshes gaat van 27 ms naar
ongeveer 2,5 s bij het opstarten.

## Controle zonder GPU

- `opaque_crown_places_one_twig_per_frond_on_its_card`: 136 onderdelen per
  boom, hout onveranderd, alle vertices eindig en binnen de kaarten (5%
  marge), geen enkele triangle tegen zijn normaal in.
- `opaque_crown_silhouette_matches_alpha_tested_cards`: orthografische
  CPU-maskers van de fijnste bron uit 16 richtingen. Bedekking van de
  geometrie tegen de alpha-geteste kaarten (drempel 0,42): 0,965 tot 0,977;
  slechtste IoU 0,815 tot 0,858 per archetype. Naalden zijn op deze schaal
  subpixel, dus de IoU is pessimistisch; de bedekking is wat een kijker op
  afstand ziet.
- Lavapipe-shots (`sideways shot`, 640x360 en 960x540, zonder verre bossen):
  kaarten en geometrie zijn visueel gelijkwaardig; gemiddeld 5,2/255
  verschil per kanaal.

## Meten op de 5070 Ti

`games/sideways/scenarios/opaque-crowns.toml` (native 1080p, één view, twee
camera's: in het bos en boven het bladerdak) met drie omgevingsbestanden:

    python engine/tools/run-frame-scenario.py games/sideways/scenarios/opaque-crowns.toml --exe <sideways.exe> --out <nieuwe map> --hidden --env-file games/sideways/scenarios/opaque-crowns.cards.json
    (idem met opaque-crowns.opaque.json en opaque-crowns.opaque-traversal.json)

Te verwachten risico's, nog niet gemeten:

- **Cull.** Met ongeveer 4.640 clusters per boom wordt de vlakke clusterlus
  per owner ongeveer 55 keer zo lang. Daarom de variant met
  `KERN_CLUSTER_TRAVERSAL=1`: pas nu heeft de r46-doorloop genoeg clusters om
  iets op te leveren.
- **Kleine triangles.** Dunne naaldgeometrie wordt snel subpixel; zonder
  software-rasterizer betaalt de hardware quad-overshading.
- **Vereenvoudiging.** Dunne vormen verliezen oppervlak bij simplificatie
  (Epics "triangle soup"). Op middelgrote afstand kan het bos dunner worden.
  Dat is het punt waar voxels per cluster in beeld komen.
- **Schaduw.** Naaldschaduw rastert dezelfde geometrie; de dynamische
  schaduwraster kan sterk groeien.

Alleen als de geometrievariant op de echte GPU niet trager is dan kaarten, of
de traversalvariant dat oplost, is de volgende stap zinvol: geïnstantieerde
onderdelen tijdens runtime (echte assemblies, minder geheugen) en voxels als
laatste clusterniveau.
