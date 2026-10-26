# Bosrendering in Warp

Actuele aanvulling, **26 oktober 2026, r51**:
[ondoorzichtige naaldkronen en de afstand tot Nanite Foliage](opaque-crowns-r51.md).
De naaldkaarten kunnen worden vervangen door echte geometrie: één gebakken
takonderdeel (1.937 triangles) per frond, zonder alpha test, achter
`SIDEWAYS_TREE_CROWN=opaque`. CPU-silhouet en lavapipe-beeld komen overeen met
de kaarten; GPU-kosten zijn nog niet gemeten. Standaard blijft `cards`.

Actuele aanvulling, **26 oktober 2026, r49**:
[LOD-optiek en vraaggestuurde schaduwen](lod-shadow-demand-r49.md).
De kleine schaduwproef behoudt gevraagde dieptetexels en eindbeelden exact bij
minder ingediende triangles. In de echte vier-spelerscene vraagt de proef alle
tegels aan: B is trager en D's fijnere receivergrenzen veroorzaken zelfs 89–92 ms
extra gatherwerk. E begrenst dit zodra kaarten volledig aangevraagd zijn,
behoudt exact beeld/diepte in vijf kleine fixtures, maar kost in de echte scene
nog 2,71–2,86 ms extra zonder casterreductie. Het blijft standaard uit.
De voxelgrasproef vergelijkt
dezelfde bronplanten: de volledige bewegende LOD-proef verbetert mediane
dekkingsfout van 7,92% naar 5,73%, maar lage-zonhelderheid wijkt nog tot 39,24%
af. Nabije voxels blijven te onscherp; geen live vervanging geaccepteerd.
De huidige grote bosbenchmark gebruikt nog kaartgeometrie/CLOD, geen automatische
voxelvervanging: 55,05 miljoen boomtriangles over vier camera's. Volumegroepen die
bronowners vóór clusterselectie/raster vervangen blijven een ontbrekende
structurele stap. Dit is geen afgeronde Nanite- of VSM-implementatie.

Actuele aanvulling, **26 oktober 2026, r48**: [echte lege-voxel-sprongen en gedeelde koplampselectie](render-traversal-r48.md). Het kleine, onveranderlijke GPU-voxelveld doorloopt 85,32% minder stappen en integreert 13–14% sneller met exact dezelfde pixels; het is nog geen geaccepteerde vervanger van het speelgras. Koplampselectie via de ruimtelijke hiërarchie bespaart in vier-view-ABBA 2,36 ms 's nachts (34,35→31,99 ms) en 2,49 ms bij zonsopkomst (46,92→44,43 ms), met één 4K-uitvoer/DLSS Quality en dezelfde dichtheid, LOD en schaduwresolutie. Schaduwkaarten en geselecteerde casters zijn exact gelijk; strikte beeldgelijkheid faalt ook tussen herhaalde referentieruns. Beide functies blijven experimenteel en standaard uit. Het [plan voor schaduwselectie vanuit ontvangers](receiver-shadow-demand-plan.md) richt vervolgwerk op het grote dynamische zonrasterbudget; dat plan is nog niet geïmplementeerd. Geen interactieve playtest actief; 8,33 ms niet gehaald.

Actuele aanvulling, **26 oktober 2026**: [r47 — minder dubbel graswerk en echte optical bricks](meadow-work-r47.md). De vier-view-ABBA meet 4,86 ms winst rond de middag en 2,85 ms bij zonsopkomst door één grasgeometriepass te vervangen door visibility + resolve, met dezelfde bron. Strikte pixelgelijkheid faalt; het blijft opt-in. Het echte voxelprototype slaagt mechanisch maar is visueel en qua kosten nog niet geaccepteerd. [r46 — renderwerk en ontbrekende technieken](render-work-avoidance-r46.md) bevat de voorafgaande schaduw- en BVH-proeven. Historische metingen hieronder blijven als referentie behouden.

Werkplan, bijgewerkt op **25 oktober 2026**. Doel: een dicht, gevarieerd en bewegend bos, met herkenbare planten van dichtbij tot aan de horizon en belichting die klopt terwijl de speler rijdt. Engineontwikkeling en toepassing in Sideways zijn beide onderdeel van ieder mijlpaalresultaat.

De ambitie is **3840 × 2160 uitvoer, vier spelers en 120 echte frames per seconde**. Dat doel is **niet behaald of aangetoond**. Native rendering en DLSS krijgen afzonderlijke resultaten. Een technische demo van Unreal is een kwaliteitsreferentie, geen bewijs dat dezelfde combinatie van functies binnen ons budget past. Epic beschrijft de Witcher 4-presentatie als een technische demo op PS5 met 60 FPS en ray tracing; dat is geen vier-spelerbenchmark of uitgegeven spel. [Epic over de demo](https://www.unrealengine.com/news/all-the-big-news-and-announcements-from-the-state-of-unreal-2025).

## Architectuuraudit richting 8,33 ms — 25 oktober 2026

Deze broncode-audit vergelijkt de werkelijk aanwezige paden met primaire Epic-documentatie. De volgorde hieronder is een werkprioriteit, geen voorspelde snelheidswinst. Eén 4K-uitvoer met vier splitscreenviews bevat samen ongeveer 8,29 miljoen uitvoerpixels; selectie, geometrie, schaduwen en buffers kunnen wel per view herhaald worden. Native 4K en DLSS Quality blijven afzonderlijke meetgevallen. Oude r28/r39-resultaten verderop zijn historische nulmetingen, geen actuele benchmark.

### Historisch r41-vier-viewbewijs, niet de huidige runtime gemeten

De gecontroleerde r41-nachtvergelijking is recenter dan de r39-cijfers: [volledig rapport](C:/dev/perf/warp-sideways-20261025/r41-night-volume/four-a/report.json), [uitvoerbaar bestand en configuratie](C:/dev/perf/warp-sideways-20261025/r41-night-volume/four-a/candidate-r41/run.json). Kandidaat-SHA256 `2af5b2fc755741c3da23590622358d3a7509c95f2b5c0714fe0d4bfd22ef45bb`; één 3840×2160-uitvoer, DLSS Quality zonder fallback, vier views van elk 1277×717 renderpixels naar 1916×1076 uitvoerpixels plus gutters. Er zijn 71 gekoppelde schone GPU-samples na 24 warmupframes; de capture is uitgesloten. Wind staat vast op sterkte 0,6 en tijd 1,9 s. Dit is één nachtsectie van die executable, geen native-4K-meting of validatie van de nieuwste debug-UI-build.

| Gemeten scope | Gemiddelde r41-kandidaat |
| --- | --- |
| GPU renderinterval, eerste gemeten begin tot laatste einde | 52,474 ms |
| Meadow-brongeometrie, `meadow render` | 16,248 ms |
| Resterende bomen raster, `tree remaining raster` | 10,338 ms |
| Zon-schaduwraster, `shadow raster` | 5,724 ms |
| Hoofdshading, `shade` | 5,085 ms |
| Resterende bomen selectie, `tree remaining cull` | 3,424 ms |
| Grasatlas, `meadow aggregate render` | 2,963 ms |

Passintervallen zijn diagnostische scopes, geen optelbare categorie-attributie of afzonderlijke FPS. GPU-HUD/presentation vallen buiten het renderinterval; eventuele overlap/gaten zijn niet als CPU-werk te interpreteren. De gemeten CPU-framewalltime van 52,642 ms bevat wachten en is **geen CPU-uitvoeringstijd**. Actieve CI kan concurreren; geen betrouwbaarheidsinterval of performancecertificering. De r40-referentie binnen dezelfde proef meet 52,400 ms GPU-interval: deze nachtverlichtingwijziging toont geen betekenisvolle snelheidswinst. Nieuwere r43-debugtests gebruiken één gepauzeerde native1080p-camera met wisselende controls en zijn geen vier-spelerbenchmark. Nieuwe optimalisaties moeten opnieuw met dezelfde scopes worden gemeten.

### Gemeten voortgang r44: minder zonwerk en minder bladen per pol

De [r44-resultaten en beeldbeoordeling](meadow-budget-r44.md) vullen dit historische beeld aan. Bij exact nul direct zonlicht worden de zonne-schaduwpassen nu overgeslagen en blijven hun textures beschikbaar voor hergebruik. De gecontroleerde vier-view-ABBA met één 4K-uitvoer/DLSS Quality meet 's nachts **47,462 / 47,284 ms uit tegenover 42,851 / 42,905 ms aan**, 95 schone samples per sectie en run. Zonsopkomst herstelt de passes; kleine verschillen overdag vallen binnen herhaalvariatie. De gezamenlijke proef bewijst geen afzonderlijke allocatiewinst onder bewegende casters en bereikt geen 8,33 ms.

Gras heeft daarnaast een expliciete **360 / 180 / 120-bladenreceptkeuze**, met respectievelijk 1× / 2× / 3× bladbreedte en behoud van alle geschikte wereldwortels. Atlas en substraat worden uit dat recept opnieuw gemaakt. De native1080p-proef met twee vaste middagcamera's meet GPU-medianen **18,461 / 20,222 ms** voor 360, **16,300 / 17,969 ms** voor 180 en **15,038 / 16,880 ms** voor 120; telkens 127 schone samples. Beide reducties houden in de beoordeelde JPEGs de brede bedekking, terwijl de eerdere halve-wereldwortelproef kale banen maakte. 180 is het visuele compromis; 120 toont duidelijk grovere lintbladen en is gekozen als expliciete performance-speelinstelling voor vervolgcontrole. De bronstandaard blijft 360 tot bewegings- en gebruikersacceptatie.

De afzonderlijke [vier-view-ABBA voor 360 tegenover 120 bladen](meadow-budget-r44.md#blade-budget-four-view-dlss-abba-and-visual-tradeoff) is nu voltooid: **52,106 / 52,093 ms** tegenover **43,312 / 43,319 ms** GPU-renderinterval, ongeveer **17% lager**, met 127 schone samples per run. Het bron-grasscope daalt van **21,855 / 21,834** naar **13,264 / 13,209 ms**. Dit is één 4K-uitvoer, vier onafhankelijke camera's en DLSS Quality van 1277×717 naar 1916×1076 per view; geen native-4K-rendering of 8,33ms-resultaat. Alle vier middag-JPEGs zijn beoordeeld: brede bedekking blijft, maar dichtbij zijn de bredere hoekige lintbladen duidelijk zichtbaar. `SIDEWAYS_MEADOW_BLADE_BUDGET=120` is een expliciete kandidaatinstelling; de standaard 360 blijft behouden. Wind-/bewegingsacceptatie loopt apart. Deze winst mag niet opgeteld worden bij de nachtelijke schaduwwinst uit een andere proef.

### Gemeten voortgang r45: hiërarchie en cachefundamenten

Drie begrensde engineproeven zijn geïmplementeerd, met afzonderlijke tellers, geheugenbudgetten en dezelfde brongeometrie als referentie. **Geen daarvan is een volledige Nanite- of VSM-implementatie.** De bevroren GPU-kandidaat heeft SHA256 `58367b0d9025237378115bda3074f32f4629f42c2e7ac96ab911af4d922f6538`; 422 renderertests en 164 gametests slagen, met ieder één bestaande genegeerde test. Dit bewijst geen optische of temporele gelijkheid.

| Enginefunctie | Werkelijk resultaat | Besluit |
| --- | --- | --- |
| [GPU-hiërarchie van ruimtelijke instantiegroepen](instance-hierarchy-r45.md) | Actuele windbounds worden eenmaal per frame bijgewerkt; iedere camera selecteert eigen groepen vóór de bestaande owner/clusterselectie. Vier-view-ABBA met één 4K-uitvoer/DLSS Quality meet **43,259→42,574 ms** (0,685 ms / 1,58% lager), 127 schone samples per run. Geometrieselectie is exact gelijk en de veiligheidsfallbacks blijven nul. De strikte pixelvergelijking faalt nog op kleine beeldverschillen; herhaalde referentieruns verschillen ook. | Experimenteel, globale standaard uit. De beperkte winst verwijdert niet het overheersende gras- en schaduwrasterwerk. De afzonderlijke wind/cameracutproef behoudt geometrieselectie, maar bereikt ook geen strikte pixelgelijkheid. |
| [Gescheiden statische/dynamische zoncache](static-sun-cache-r45.md) | Statische diepte kan bij geldige projectie worden hergebruikt en met actuele dynamische casters gecombineerd. 98–99% cachehits leveren in het bos **geen nuttige gehele-framewinst** op; bewegende bomen domineren, bij zonsopkomst 52,66 miljoen dynamische versus circa 54 duizend statische driehoeken. Extra geheugen circa 115,69 MiB. | Uit. Volgende stap is zichtbaarheids- en resolutiegestuurde schaduwpagina's voor actuele windposes, niet een grotere cache voor goedkoop statisch werk. De onafhankelijke projectieguard blijft ook uit. |
| [Gedeelde wereldpagina's voor grasgroepen](meadow-group-cache-r45.md) | Richtinggebonden, opnieuw belichtbare oppervlaksamples uit dezelfde plantenbron, met per-pol-fallback. De verre aggregatepass bespaart 0,108 ms, maar het gehele GPU-frame verbetert niet en CPU-planning kost extra. Slechts maximaal 2,29% van de verre referentiewortels valt onder actieve pagina's; alle referentiewortels worden nog ingediend. Extra GPU-geheugen 32,52 MiB. | Uit, uitsluitend native toegestaan. Correcte bewegingshistorie en echte vervanging van het referentiewerk ontbreken. Dit is een oppervlaktecache, geen voxelveld. |

De profiler bewaart nu ook **daadwerkelijke uitvoeringstijd van de render-hoofdthread**, naast frame-walltime. Op Windows worden kernel- en userticks gemeten; de grove OS-tijdstap vraagt een gemiddelde over meerdere frames. Wachten, descheduling en workerthreads zijn geen onderdeel van deze teller. De [frame-budgettool](frame-budget.md) houdt ontbrekende metingen apart en maakt van walltime geen CPU-uitvoering. De launcher kan een expliciet met SHA256 aangewezen executable direct starten en veilig kopiëren, zodat documentatie- of harnesswerk geen onnodige rebuild veroorzaakt.

In de r45-vier-viewmiddagscène meet de eerste hiërarchie-aanrun **13,269 ms** voor bron-gras, **6,726 ms** voor resterende bomenraster, **6,668 ms** voor hoofdshading en **5,599 ms** voor verre grasaggregatie. Zonraster kost daar **1,455 ms**; de zware zonsopkomst uit de afzonderlijke schaduwproef is dus niet representatief voor ieder tijdstip. Deze scopes zijn geen optelbare onafhankelijke kosten. Werkelijke hoofdthread-CPU-uitvoering is gemiddeld **7,997 ms** over 127 schone samples, tegenover gemiddeld **42,778 ms** frame-walltime; de grove Windows-ticks maken de per-framemediaan van nul onbruikbaar als CPU-snelheidsclaim.

De prioriteit na deze metingen is **minder daadwerkelijk gerasteriseerde brongeometrie en dynamische schaduwen**. Een cachehit of lager aantal bezoeken is onvoldoende als dezelfde wortels, vertices en shadowtriangles alsnog verwerkt worden. Het 8,33ms-doel blijft ruim buiten bereik; resultaten uit verschillende scènes mogen niet worden opgeteld.

De beperkte r45-wind/cameracutproef is inmiddels voltooid: alle twaalf capture-inputs en geometrieselecties gelijk, nul veiligheidsfallbacks, alle 24 beelden beoordeeld zonder ontbrekende bomen/terrein. Vier sky-paren zijn pixelgelijk; acht bosparen verschillen maximaal 7/255 en blijven strikt afgekeurd. Deze stilstaande camerastanden met doorlopende wind en cuts bewijzen geen volledige bewegingspariteit. De expliciete experimentele speelproef draait met `KERN_INSTANCE_GROUPS=1`, beide nieuwe caches uit, 120-bladenrecept, live wind 0,6, één 4K-uitvoerview/DLSS Quality en F3-debug-UI. [Runmanifest](../../games/sideways/build/dev-runs/20261025T224631837Z-b411005b/iteration.json); PID 232300, start-UTC-ticks 639285651926972671, bevroren kandidaat-SHA256 hierboven. Eerste zichtbare frame en responsief proces gecontroleerd; geen extra compilatie. Globale featuredefaults blijven uit.

### Wat Epic beschrijft, en wat Warp werkelijk doet

- **Assemblies verminderen ook ver werk.** Epic deelt onderdeelgeometrie en kan verre part-roots samenvoegen totdat een assembly één grof cluster wordt. Gewone instancing houdt werk per instantie over. Warp deelt primitive-meshes en branchposes in de begrensde `CookedPlant`-proef, maar heeft nog geen gezamenlijke grove assemblyselectie. Het speelbos bestaat nog uit drie alpha-cardarchetypen. [Epic Assemblies](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-assemblies), [Warp assetbuilder](../crates/render/src/vegetation_asset.rs).
- **Nanite-voxels zijn niet onze huidige multi-owner raymarcher.** Epic beschrijft clusters van maximaal 128 bricks van 4³ voxels, foutgestuurde triangle/voxelkeuze, aparte rasterbins en front-to-back-dieptevolgorde. Een normalenverdeling bewaart meerdere bladrichtingen. De Witcher-demo gebruikte directe shading van die verdeling; de huidige algemene documentatie beschrijft daarnaast een stochastische GBuffer-route. Warp heeft normale-/extinctiemomenten en een begrensd volumeprototype, maar geen equivalente schaalbare brickrasterhiërarchie. De live meadow-atlas is geen voxelvolume. [Epic Nanite Foliage](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage), [Warp volumeveldplan](meadow-volume-field.md).
- **Compute-shading ontbreekt niet.** De hoofdvisibilitybuffer wordt al per pixel in compute belicht. Het huidige gras gebruikt eigen geometrie- en aggregatepasses. Het zinvolle verschil zit in hun selectie-, vertex-, overdraw- en geheugenkosten, niet in opnieuw de hoofdlichtpass naar compute verplaatsen. [Hoofdshader](../crates/render/src/shaders/shade.wgsl), [meadow renderer](../crates/render/src/meadow.rs).
- **Gaten maken overdraw moeilijk.** Epic waarschuwt dat veel losse bladeren en lagen met gaten zowel vereenvoudiging als occlusion hinderen; masked-out pixels blijven duur. Geometrische bladeren zijn een geschikte bron, maar miljoenen opaque microtriangles zonder grove aggregatie lossen dit niet automatisch op. [Epic over aggregate geometry en foliage](https://dev.epicgames.com/documentation/unreal-engine/working-with-naniteenabled-content).

Epics feature is experimenteel. Een PS5-demonstratie op 60 FPS is geen bewijs voor onze combinatie van native 4K, vier views en 120 FPS. Groei- en soortenauthoring helpt hergebruik en variatie, maar is op zichzelf geen rendereroptimalisatie.

### Gerichte volgorde

| Prioriteit | Concrete broncodebeperking | Begrensde volgende stap en acceptatie |
| --- | --- | --- |
| 1. Werk laten dalen met schermgrootte | De standaardbron heeft 360 sprieten per pol en 1800/1080/600 triangles. r44 heeft ook gemeten opt-inrecepten met 180 of 120 bredere sprieten; dit verlaagt bronwerk maar is nog geen gezamenlijke verre groepselectie. De atlas verwerkt nog aparte pollen en extra overlap-/resolvepasses. Het algemene volumeprototype is geen wereldveld. | Maak gezamenlijke grove representaties uit dezelfde bron, met richtingafhankelijke dekking, normaleverdeling en materiaal. Selecteer op geprojecteerde fout; tel bezochte groepen/bricks, geometrie en alle extra passes. Geen groene bodemkleur of willekeurig verdwijnende pollen als equivalent presenteren. [r44-receptproef](meadow-budget-r44.md#blade-budget-inside-every-clump-native-results), [Bron-LOD](../crates/render/src/meadow_data.rs), [atlasgrenzen](meadow-distance-r39.md). |
| 2. Hele onzichtbare subtrees overslaan | r45 voegt een experimentele GPU-hiërarchie van ruimtelijke instantiegroepen toe, met actuele windbounds en eigen zichtbaarheid per view. De overgebleven `cull_instance`-selectie loopt nog langs alle residentiële LOD-clusters; top-down clustertraversal ontbreekt. Fase-2-bomen occluderen elkaar niet. | Rond temporele controle van de groepen af en richt verder werk op werkelijk bezochte clusters en rasterwerk. Behoud camera-cut-, wind- en split-pariteit; het huidige 1,58%-resultaat is geen groot rasterbesparingsbewijs. [r45-hiërarchie](instance-hierarchy-r45.md), [Culler](../crates/render/src/shaders/cull.wgsl), [tree Hi-Z](forest-occlusion-r38.md). |
| 3. Raster- en vertexwerk verminderen | Live boomkronen zijn alpha-cards. Meadow heeft reeds indexed draws, een diepteprepass en gecachte wind per branch; die twee geometriepasses verwerken nog veel dunne brontriangles. Hoofdshading is al compute. | Voer opaque herbruikbare onderdelen samen met grove aggregatie in. Een gespecialiseerde microtriangle-rasterizer is pas verantwoord als gemeten raster-/vertexwerk de bouwkosten rechtvaardigt. Meet quad-/fragmentwerk naast triangles; minder triangles kan extra shading of compositing kosten. |
| 4. Animatie en schaduwcache samen begrenzen | Branchposes worden in de proef gedeeld; het grote speelbos gebruikt instance-shear. De gedeelde foliage-schaduwpagina's zijn CPU-requested en gebruiken verre rustposes, geen volledige dynamische VSM. | Animatie-LOD op zichtbare fout, poses eenmaal voor de vereniging van views, strakke actuele bounds. Daarna alleen waar de meting het vraagt: depthgestuurde pageaanvragen, resolutie-LOD en gescheiden statische/dynamische invalidatie. Bewaar offscreen zonblokkeerders. [Wind](../crates/render/src/part_wind.rs), [pagina's](../crates/render/src/foliage_sun_pages.rs), [Epic VSM](https://dev.epicgames.com/documentation/en-us/unreal-engine/virtual-shadow-maps-in-unreal-engine). |
| 5. Residency delen in plaats van radius vermenigvuldigen | De r39-atlaspool reserveert circa 77 MiB per view bij 180 m; vier views circa 308 MiB, nog zonder extra beeldtargets. Dit is een begrensde implementatie, geen compact wereldclipmap. | Deel wereldvaste bronpagina's, cooked assets en poses; houd viewlijsten en LOD-selectie apart. Rapporteer daadwerkelijke residentie, uploads, targets en cachemisses. Meer drawdistance zonder deze structuur kan het probleem vergroten. [Geheugenbudget](meadow-distance-r39.md#allocation-and-work-budget). |

Epics skinning gebruikt matrices voor strakkere bounds en stopt kleine animatie mede voor schaduwcachekosten. Dit ondersteunt animatie-LOD, maar rechtvaardigt niet het stil bevriezen van zichtbaar bewegende bladeren. Epic VSM vraagt pagina's vanuit zichtbare diepte en kan statische/dynamische cachelagen scheiden; onze huidige rustposecache blijft een expliciete benadering. [Epic skinning](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage), [Epic schaduwcaching](https://dev.epicgames.com/documentation/en-us/unreal-engine/virtual-shadow-maps-in-unreal-engine).

### Kleine grove-grasproef vóór een nieuwe renderer

Een hogere `meadow_lod_pixels` kiest alleen uit de bestaande drie bron-LOD's. Als vrijwel alle pollen al 600 triangles gebruiken, is daarvan nauwelijks verdere geometriewinst mogelijk. Alle 360 bladen tot één triangle reduceren zou in die toestand hoogstens 40% van de brondriehoeken sparen, niet 40% van het hele frame. De huidige richtingsafhankelijke oppervlakteguard verwerpt die reductie juist voor één bronvorm; de guard afzwakken is geen bewezen optimalisatie. [Projectieguard en foutgrenzen](../crates/render/src/vegetation_clod.rs).

De afzonderlijke r44-diagnose laat dezelfde bestaande atlas eerder overnemen: 40–60 m tegenover 60–80 m, met identieke roots, dichtheid, wind en 180 m buitengrens. De verre nativecamera meet **20,273→19,486 ms**, maar de nabije **18,548→18,731 ms**; extra aggregatewerk eet een deel van de bronwinst op. Dit is **geen gepromoveerde standaard** en geen voxelimplementatie. Op grotere schermvoetafdruk kunnen rustdekking, representative-depth, wind, normalegemiddelden en bermgaten duidelijker afwijken. Bewegende vier-view-, tegenlicht- en nachtacceptatie ontbreken nog voor deze overgang. De proef met minder, bredere bladen hierboven is een andere ingreep: zij behoudt wereldwortels en de 60–80 m-overgang. Een daadwerkelijke grove geometrie- of brickrepresentatie moet daarnaast openruimte en kleur over de overgang bewaren.

Iedere promotie vraagt hele-framebewijs: echte CPU-uitvoering apart van frame-walltime, gemeten GPU-interval apart van passommen, render-/uitvoerresolutie, vier viewrects, voldoende schone samples en expliciete warmup/capture/CI-beperkingen. Categorieweghaalproeven geven marginale, niet optelbare kosten. Geen percentage "Nanite af" of 120-FPS-claim op basis van alleen gedeeld geheugen of een kleine plantenfixture.

## Status en uitgangspunten

**Volgende stap — één voxelveld voor de verkenbare wereld:** de gebruiker wil
voor gras dezelfde bron en volumerepresentatie op alle grote afstanden, zonder
een aparte overgang naar geschilderde bodemkleur. De nieuwe CPU-cook maakt
echte `VoxelAsset`-mips uit de huidige 360 bronbladen. Een schaalbaar veld met
gedeelde wereldpagina’s, vier-viewresidency, onafhankelijke windgroepen en
optisch gevalideerde overgangen is nog niet geïmplementeerd. Het
[concrete volumeveldplan](meadow-volume-field.md) beschrijft de API’s,
geheugenlimieten, mijlpalen en acceptatie; onderstaande r39/r40-speelpaden
blijven de referentie totdat dat werk slaagt.

**Update r39 — verre grasdekking, speelproef met zichtbare beperkingen:** een
directionele atlas uit dezelfde grasbron houdt detail tot180m;60–80m mengt
bronbladen naar de goedkope laag,160–180m naar bijpassende bodemkleur. Dit is
geen voxelvolume. Hardwarefiltering en één belichtingsberekening per pixel
vervangen de afgewezen trage eerste kandidaat. De bronvorm en dichtheid blijven.
De twee F12 native1080p-views kosten19,041→20,240 en18,451→19,110ms. Vier spelers
met één4Koutput/DLSSQuality kosten in twee korte secties ongeveer gelijk:
50,566→50,557 en60,580→60,377ms;71 schone GPU-samples per sectie.120FPS blijft
onbehaald. De dekking is beter, maar op open verre hellingen blijft een licht
rasterpatroon en langs de weg blijven getrapte grasranden zichtbaar. Alle22
native stills en48 motion-JPEGs beoordeeld; geen bewijs van volledige temporele
pariteit. De nieuwe speelproef draait vanuit
`games/sideways/build/dev-runs/20261025T183451763Z-519c1346`.
[r39-metingen, technieken en beperkingen](meadow-distance-r39.md).

**Update r38 — speelproef met beperkte acceptatie:** kronen zijn voller door
stemgebonden verbreding van dezelfde136fronds/1088kroondriehoeken per bron.
Boomaantallen en fijnste brontriangletellingen blijven gelijk; herbouwde CLOD
kiest wel andere geometrie. De hoofd-culler heeft nu een actuele tweefasenroute:
eerst echte alpha-geteste terrein-/nabije-boomdiepte, daarna conservatieve
Hi-Z-selectie van overige bomen. Schaduwculling blijft onafhankelijk en iedere
view houdt eigen zichtbaarheid. Fase2-bomen occluderen elkaar nog niet onderling.
Het live bos gebruikt nog drie gedeelde alpha-cardbronnen, geen volledig
procedureel assembly-/voxelbos.

Statische native pariteit6/6 en vier-viewpariteit2/2 zijn exact. De voorlopige
`performance-b`-meting heeft39 schone GPU-samples per camera, één4Koutputview
met DLSSQuality: culling uit→aan24,784→23,974ms bos,17,165→10,756ms klif en
34,821→27,858ms wegpanorama. Geen betrouwbaarheidsinterval of120FPS-claim.
**Temporele acceptatie is niet geslaagd:** twee herhalingen verschillen op één
pixel(217,232) in frame22, tree→terrain, maximale kanaalfout62;31/32 PNG-paren
zijn exact en alle96 inputs/checksums gelijk. Off/off is exact. Een dieptetie
door tekenvolgorde is plausibel, maar nondeterminisme of de oorzaak is niet
bewezen. De exacte pixelgate blijft gefaald.

Op verzoek van de gebruiker draait de bestaande geteste release met
fullness1.25/culling1: PID267312, startUTCticks639285453706207673, SHA256
`efb28754f4223aa1fb9258944308467b375848dc29447378d7d989b12e9b5c04`.
De root verifieerde een responsief proces, constructielogs en eerste zichtbare
frame; een eerdere launcher-verificatiefout leidde niet tot een tweede restart.
[Actief runmanifest](../../games/sideways/build/dev-runs/20261025T171609647Z-r38-play/iteration.json).
Eén3840×2160output, DLSSQuality en livewind0,6. Nieuwe broncode voor
`KERN_TREE_OCCLUSION_REJECT=0` plus `rejection_enabled`-metadata is nog niet
gebouwd/gevalideerd en zit niet in deze executable. Deze diagnostiek houdt
beide fasen maar schakelt alleen Hi-Z-verwerping uit; geen pixelmarge versoepeld.

De nieuwste melding van gras-pop/screenshots wordt onderzocht. Verre
grasaggregatie, eigen grasschaduwen en rand-/popproblemen blijven open.
[r38-resultaten, fout en beperkingen](forest-occlusion-r38.md).

**Update r37:** fysieke foutgrenzen laten dezelfde grasvorm eerder naar zijn
eenvoudigere bron-LOD gaan; exacte indexbuffers vermijden dubbel vertexwerk.
De360sprieten per pol blijven. In de gemeten rijcamera32,63%minder
brondriehoeken en grasGPU15,368→12,619ms(-17,89%), met dezelfde dekking in
de beoordeelde beelden. Native pixeltests oefenen alle drieLOD-niveaus en
blijven exact gelijk met indexering aan/uit. Eén korte4K/DLSSQuality A/B
onderCI; geen120FPS-claim. Verre voxelaggregatie blijft de volgende grote
vereenvoudiging en is nog niet geïmplementeerd.
[Metingen, speelproef en vervolgrichting](meadow-budget-r37.md).

**Update r36:** na afwijzing van de r35-dekking heeft ieder vast graspatchje
360 in plaats van36 smalle sprieten. De wortels zijn over0,45×0,45m verdeeld,
blijven vast bij groei/wind en gebruiken bron-LOD1800/1080/600 driehoeken.
Echte beeldvergelijking bevestigt veel vollere bedekking; sommige vierkante
bermranden blijven. GrasGPU in de korte chasevergelijking11,776→15,261ms;
dit is een dichtheidsverbetering, geen performancewinst. Verre voxelaggregatie,
eigen grasschaduwen en het120FPS-doel blijven open.
[Nieuwe speelproef, aantallen en bewijs](meadow-density-r36.md).

**Update r35:** gras heeft nu stabiele groene/droge kleurplekken en meer lage
bladbedekking. De GPU laat de bronvorm en wind de lokale terreinhelling volgen;
een fout in de richting van de bladdoorsnede is hersteld. De derde echte
beeldvergelijking is bruikbaar voor een nieuwe speeltest, met zichtbare grenzen:
de laag in volle zon oogt nog vlak en eigen grasschaduwen ontbreken. Geen extra
bladen per pol, geen verre voxelaggregatie of 120FPS-claim.
[Beelditeraties en bewijs](meadow-look-r35.md).

**Update r34:** de gedeelde grasbron wordt nu over het terrein toegepast in
`SIDEWAYS_PROCEDURAL_GRASS=field`. GPU-plaatsing uit terrein/seed, drie varianten,
vier windgroepen per pol, bron-LOD, DLSS-motion en conservatieve actuele
diepte-occlusie zijn geïntegreerd. De opt-in vervangt fotografisch gras.
Verre voxelaggregatie en eigen grasschaduwen ontbreken nog; de huidige
veldgrens is 100 m. De Hi-Z-controle behoudt exact dezelfde pixels in de
native off/on/off-proef. CPU-timings zijn beïnvloedbaar door gelijktijdige CI;
geen 120 FPS-claim. [Bewijs en beperkingen](meadow-field-r34.md).

**Update r32:** de gedeelde procedural proef heeft nu tussenliggende bron-LOD voor gesloten grasbladen/naalden en een correctie voor lege-ruimtesprongen bij de gekozen voxel-mipgrens. Bij 150 m daalt het geselecteerde werk per 1080p-view van 6.960 naar 4.152 kleurdriehoeken; 72 beeldvergelijkingen per plant zijn geslaagd, met exact dezelfde beelden op 5 m. Een afzonderlijke ABBA-proef meet minder GPU-werk voor de voxelcorrectie. Dit betreft zes proefplanten, geen volledig bos of gameplay-FPS; optische source/voxelpariteit en DLSS voor dit pad blijven open. [Meetopzet, resultaten en grenzen](vegetation-optimization-r32.md). De eerdere r31-metingen blijven als historische nulmeting bewaard.

**Speelpad** betekent aanwezig en toegepast in eerdere lokale playtests, niet foutloos of commercieel uitgebracht. **Proef** betekent opt-in of alleen een fixture. **Actief** betekent werk in uitvoering zonder voltooide acceptatie. **Ontbreekt** betekent geen bruikbaar geïntegreerd pad.

**Historisch lichtbewijs r25:** de lichtdemo bereikte een bevestigd eerste zichtbare frame met 3840 × 2160 output, één view, DLSS Quality en wind `0.6`. [Runmanifest](../build/dev-runs/20261024T175235340Z-a3048d19/iteration.json), executable-SHA-256 `2c3912c46d8baa213383f658859aaf43fb577e12d39dd79a3ba59d9c457f7b48`. Deze speelproef gebruikte `KERN_WORLD_TERRAIN_VISIBILITY=1`, `KERN_PAGED_FOLIAGE_SUN=1`, `KERN_FOLIAGE_SUN_PAGE_METRES=256` en `KERN_FAR_SUN_SHADOW=0`. Het manifest bewijst die run, geen momenteel draaiend proces. Indirect licht gebruikt nog het heuristische pad; de radiance-cacheproef levert **geen live GI**. De nieuwste prestatierichting volgt uit de onderstaande r28-meting.

De eerdere r24-run, PID `235264`, is normaal door de gebruiker gesloten (`window_closed`); het [oude manifest](../build/dev-runs/20261024T173645295Z-replay-stable/iteration.json) blijft alleen historische herkomst.

We bouwen eigen technieken rond onze meetbare problemen. We claimen geen volledige Nanite, Lumen of Virtual Shadow Maps en kopiëren geen Unreal-code. De eerdere vergelijking met Godot is afgesloten; validatie vergelijkt Warp-versies en onafhankelijke bronreferenties.

| Onderdeel | Status | Wat er werkelijk staat | Nog nodig en bron |
| --- | --- | --- | --- |
| Clustergeometrie en CLOD | Speelpad | Meshopt-clusters, hiërarchische foutselectie, GPU-frustumculling, indirecte draws en visibility buffer. Attributen en beschermde terreincorridors helpen bronvorm behouden. | Geen volledige virtuele geometrie met disk-page streaming. [cluster.rs](../crates/render/src/cluster.rs), [cull.wgsl](../crates/render/src/shaders/cull.wgsl). |
| Boomgeometrie | Speelpad | Nabij en ver gebruiken dezelfde bronbomen, materialen en gedeelde schaalpolicy. Kronen bevatten nog alpha-cutoutvlakken. | Opaque bladeren/naalden uit herbruikbare onderdelen geleidelijk invoeren; soort, gaten en silhouet behouden. [tree_assets.rs](../../games/sideways/app/src/tree_assets.rs), [distant_forest.rs](../../games/sideways/app/src/distant_forest.rs). |
| Retained instancing | Speelpad | Geometrie wordt gedeeld; instancebuffers blijven op de GPU. Culling gebeurt per view/pass, geen CPU-draw per boom. | Nog geen assemblyhiërarchie die onderdelen gezamenlijk tot een verre proxy reduceert. Alleen meshgeheugen delen verlaagt het geselecteerde rasterwerk niet. [scene.rs](../crates/render/src/scene.rs), [frame.rs](../crates/render/src/frame.rs). |
| Plaatsing en soorten | Speelpad | CPU genereert deterministische habitatplaatsing; nabije/verre bomen delen schaalregels. Drie visuele archetypen, groei en seizoen worden bij stagebouw vastgelegd; relevante colliders volgen de schaal. | GPU-plaatsing uit terrein plus seed ontbreekt. Groei is geen botanische simulatie of realtime groeiproces. [vegetation.rs](../../games/sideways/sim/src/vegetation.rs), [builder.rs](../../games/sideways/sim/src/builder.rs). |
| HZB en occlusion culling | Meadow; r38-boomroute in beperkte speelproef | Actuele minimum-depthpiramide met windbounds. Bomen krijgen twee main-colour-fasen; echte alpha-openingen blijven in de occluderdiepte. Statische6/6 en split2/2 pixelpariteit exact. | Temporele1-pixelfout nog verklaren; diagnostic-only rejection-bypass nog ongebouwd. Fase2-bomen occluderen elkaar niet; geen volledige wereld-HLOD of120FPS-bewijs. [r38](forest-occlusion-r38.md), [meadow](meadow-field-r34.md). |
| Wereldstreaming en HLOD | Gedeeltelijk / ontbreekt | Gras heeft begrensde CPU-tegelresidency en een GPU-pool; stages bouwen asynchroon met laadstatus. | Geen algemene wereldcelstreaming, geometriepage-residency of HLOD-vervanging van uitgeladen bosdelen. [live_ground_cover.rs](../../games/sideways/app/src/live_ground_cover.rs), [play.rs](../../games/sideways/app/src/play.rs). |
| Huidig gras | Nieuw veld in opt-in speelproef | r34 vervangt fotografische pollen over het terrein met gedeelde procedural brongeometrie. GPU bepaalt roots, variant, bron-LOD, zichtbaarheid en wind per groep bladen; native en DLSS. | Verre voxelaggregatie en eigen grasschaduwen ontbreken; brongeometrie is op open terrein nog duur. Huidige grens 100 m. [Meadow-architectuur en cijfers](meadow-field-r34.md). |
| Gedeelde plantaardige bron en voxels | Proef, toegepast op gemengde patch | `CookedPlant` bewaart unieke primitives; `SceneVegetation` deelt meshes over planten en verbindt members aan één branchpose/LOD-besluit. GPU scatter past de gedeelde pose toe. r30-stageproef: 23 graspollen + 2 jonge coniferen, 966 primitive-instances en 102 volume-owners. Bronbeelden bij drie windstanden zijn exact gelijk aan het onafhankelijke expanded-orakel. | Native-only, standaard uit. Gezamenlijke grove hiërarchie, optische voxelpariteit en schaalbare kosten ontbreken nog; 256 poses/owners is een proeflimiet. [API, bewijs en grenzen](unified-vegetation.md). |
| Wind | Speelpad + proef | Huidig pad berekent één shearcoëfficiënt per instance op GPU, gedeeld tussen views. Proef berekent hiërarchische branchposes met huidige/vorige matrices; bron en volume delen pose en bounds. | Volledige bosanimatie-LOD, stabiele source/voxel-motionvectors en DLSS voor part-wind ontbreken. Statistische verre graslaag beweegt niet als individuele bladen. [wind.rs](../crates/render/src/wind.rs), [part_wind.rs](../crates/render/src/part_wind.rs), [motion.rs](../crates/render/src/motion.rs). |
| Directe zon op terrein | GPU-gevalideerde opt-in; speelproef actief | Gedeelde min/max-hiërarchie traceert echte hoogtevelddriehoeken. Hit, bewezen clear en unknown blijven verschillend; geen vaste 50 m horizon. In de vastgelegde lichtdemo ingeschakeld. | Kosten amortiseren en ambiguïteit tussen verfijnde rendergeometrie en fysiek terrein bewaken. Geen mesh-SDF of volledige scene-tracer; geen algemene standaardpromotie. [terrain-world-visibility.md](terrain-world-visibility.md). |
| Verre boomschaduwen | GPU-fixture en gerichte replays geslaagd; speelproef actief | World-fixed, gecachte source-alpha-schaduwpagina’s: vastgelegde demo gebruikt 256 m pages. Noon-replay heeft 151/151 pages resident en een probleemkroon dicht bij een onafhankelijke bronschaduwreferentie; sunset-approach heeft op vier afstanden 154/154 pages resident. | Volledige bosoptische kalibratie, verre restpose onder wind en volledige vier-viewrouteprestaties blijven open. CPU requests en gedeelde pagepool: geen volledige VSM. [foliage_sun_pages.rs](../crates/render/src/foliage_sun_pages.rs), [shader](../crates/render/src/shaders/foliage_sun_pages.wgsl). |
| Huidige indirecte belichting | Speelpad, bekende tekortkoming | Globale sky-SH, canopy-occlusie en heuristische grondbounce. | Geen lichttransport via werkelijk geraakt materiaal. Directe zonblokkering mag niet alle indirecte bounce onderdrukken. [shade.wgsl](../crates/render/src/shaders/shade.wgsl), [canopy.rs](../crates/render/src/canopy.rs). |
| Radiance cache | CPU + GPU-proef geslaagd | Echte eenmalige diffuse reflectie in analytische CPU-scenes; GPU bemonstert gedeelde SH9, afstandsmomenten en geldigheid. Negen numerieke gevallen, lege cache en vier gedeelde panelen PASS. | Geen gameplay-GI, GPU-transport of volledige scenematerialen. [radiance-cache.md](radiance-cache.md), [radiance_cache_gpu.rs](../crates/render/src/radiance_cache_gpu.rs). |
| Surface cache en scenequeries | Ontbreekt; ontwerp beschikbaar | Terreinquery levert geometrie, nog geen werkelijk materiaal op het raakpunt. | Gedeelde materiaalevaluatie, surface-atlas, scene-occluders en revisies nodig vóór live bounce. [live-terrain-bounce-plan.md](live-terrain-bounce-plan.md). |
| Foliage-optiek | Speelpad + proef | Grass-bladnormalenverdeling, bronafhankelijke dichtheid en gedeelde zon/koplamprespons; procedural thin-leaf reflectie/transmissie en voxel-normalenmomenten. | Richtingsafhankelijke openruimte, overlap, zelfschaduw en energie moeten equivalent blijven bij iedere representatie. Geen globale tint/dichtheidscorrectie als vervanging. [grass_lighting.wgsl](../crates/render/src/shaders/grass_lighting.wgsl), [foliage_voxel.rs](../crates/render/src/foliage_voxel.rs). |
| Lucht, mist en lampen | Speelpad, deels opt-in | HDRI/procedurele atmosfeer, live tijdinstellingen, gedeelde zonnepositie, eindige afstandsmist, terrain-only solar-haze-occlusie en schaduwende koplampen. | Geen volledige schaduwende froxelmist, wolken of dynamisch multibounce-licht van lampen. [live-sky-controls.md](live-sky-controls.md), [finite-distance-fog.md](finite-distance-fog.md), [car-headlights.md](car-headlights.md). |
| Ontwikkellus en bewijs | Speelpad | Build/staging/restart, F12 beeld + toestand + GPU-passtijden, exacte replay, transactionele shaderreload van actieve consumers. | Iedere mijlpaal moet in Sideways zichtbaar worden; een los voorbeeld is onvoldoende eindresultaat. [tools](../tools/README.md), [capture_replay.rs](../../games/sideways/app/examples/capture_replay.rs), [shaders.rs](../crates/render/src/shaders.rs). |

## Correcties op het aangeleverde techniekoverzicht

- **Nanite Foliage is een combinatie, geen voxeltruc op afstand.** Epic beschrijft assemblies, foutgestuurde triangle/voxelkeuze en skinning; voxelshading bewaart een normalenverdeling. De huidige documentatie noemt de functie experimenteel en vermeldt ook ruis en animatiegrenzen. Dit rechtvaardigt geen absolute claim van nul pop-in of nul overdraw. [Nanite Foliage](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage).
- **Retained rendering betekent niet dat het hele bos permanent resident moet zijn.** Nanite streamt detail op aanvraag en heeft praktische grenzen. World Partition beheert wereldcellen; HLOD kan uitgeladen cellen vervangen met verschillende proxytypen. Dat zijn afzonderlijke verantwoordelijkheden. [Nanite](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-virtualized-geometry-in-unreal-engine), [World Partition](https://dev.epicgames.com/documentation/en-us/unreal-engine/world-partition-in-unreal-engine), [HLOD](https://dev.epicgames.com/documentation/en-us/unreal-engine/world-partition---hierarchical-level-of-detail-in-unreal-engine).
- **Vorige-frame-occlusie mag nieuwe zichtbaarheid niet verwijderen.** De tweefasenaanpak bouwt actuele diepte uit eerder zichtbare geometrie en test daarna ook nieuw zichtbare geometrie. Een oude dieptetest alleen is onvoldoende. [Karis, SIGGRAPH 2021](https://advances.realtimerendering.com/s2021/Karis_Nanite_SIGGRAPH_Advances_2021_final.pdf).
- **Wind hoeft niet via algemene WPO of Pivot Painter.** Nieuwe Nanite Foliage gebruikt bone-skinning voor bruikbare bounds en animatie-LOD. Pivot Painter 2 is een andere workflow: pivot/hiërarchiedata staan vooral in textures, benaderd via UV’s; niet simpelweg alleen in vertexkleuren. [Nanite Foliage](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage), [Pivot Painter 2](https://dev.epicgames.com/documentation/en-us/unreal-engine/pivot-painter-tool-2.0-in-unreal-engine).
- **VSM-pages bepalen waar resolutie nodig is, niet welke offscreen blockers mogen verdwijnen.** Epic gebruikt 128²-pages, caching en invalidatie door licht/geometrie/animatie. Afstandsschaduwen uit distance fields zijn een aanvullende route voor onder meer niet-Nanite-geometrie, geen universeel gratis horizonpad. [Virtual Shadow Maps](https://dev.epicgames.com/documentation/en-us/unreal-engine/virtual-shadow-maps-in-unreal-engine).
- **Lumen vereist naast visibility ook materiaal en licht op het raakpunt.** Surface-cache-cards zijn opnamevlakken voor oppervlaktegegevens, geen simpele boomdoos en niet dezelfde cards als alpha-bladeren. Screen traces en world tracing vullen elkaar aan. Een bewegende zon verbiedt niet alle baking: brongeometrie en materiaalgegevens kunnen vooraf worden verwerkt terwijl belichting wordt bijgewerkt. [Lumen technical details](https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine).
- **Afstandsmist is geen schaduwende volumetrische mist.** Een froxelpad moet licht en zichtbaarheid langs de zichtstraal integreren; temporele filtering introduceert bovendien trails bij snel veranderende lampen. [Volumetric Fog](https://dev.epicgames.com/documentation/en-us/unreal-engine/volumetric-fog-in-unreal-engine).

## Keuze uit Decima en Unreal

**Besluit: neem Guerrilla’s aanpak van procedurele plaatsing, visibility en gericht shaderwerk mee, en behoud onze gedeelde procedural triangle/voxel-bron.** De keuze wordt per techniek gemaakt op beeldkwaliteit, geheugen en frametijd bij vier views. Het aangeleverde Decima-overzicht is een verzameling onderzoeksideeën; het bewijst geen integraal betere engine of haalbare 4K/120-configuratie.

Wat de primaire bronnen daadwerkelijk bevestigen:

- **HZD, 2017:** GPU-plaatsing bouwt de omgeving rond de speler vanuit door artiesten gemaakte regels. Een afzonderlijke publicatie beschrijft snelle visibilityqueries en efficiënte instancebatches via async compute. Dit ondersteunt onze geplande compacte terrein/habitat/seed-input en GPU-compaction; het bewijst geen specifieke huidige meshlet-HZB-implementatie. [GPU placement](https://www.guerrilla-games.com/read/gpu-based-procedural-placement-in-horizon-zero-dawn), [visibility](https://www.guerrilla-games.com/read/decima-engine-visibility-in-horizon-zero-dawn).
- **HFW, 2022:** foliage en alpha-tested geometrie gebruiken een visibility-bufferprepass, gevolgd door analyse en shading in compute. Software variable-rate shading en overlap met cascaded-shadow-work zijn beschreven. Warp heeft al een verwante visibility-bufferbasis; de concrete vervolgvraag is hoeveel nutteloze shading en overdraw we kunnen vermijden. [Deferred texturing](https://www.guerrilla-games.com/read/adventures-with-deferred-texturing-in-horizon-forbidden-west).
- **Nubis³, 2023:** voxelwolken gebruiken onder meer compressed-SDF-versnelling voor ray marching en benaderde wolkenbelichting. Dat laat juist zien dat Guerrilla ook volumes inzet waar ze nuttig zijn. Deze publicatie gaat over wolken; zij bewijst geen forest-GI-, boomwind- of algemene distance-field-shadowpipeline. [Nubis³](https://www.guerrilla-games.com/read/nubis-cubed).

| Onderdeel | Beslissing voor Warp; nog voorstel tenzij de statusmatrix anders zegt |
| --- | --- |
| Plaatsing en residency | Genereer stabiele plant-ID’s uit wereldcellen, habitat en seed; bewaar alleen de benodigde residentie en wijzigingen. Vier views delen de vereniging van benodigde cellen. Meet CPU-winst, GPU-generatie, buffers, habitattextures en streaming samen. |
| Geometrie en wind | Herbruikbare opaque bladen/takken blijven de bron voor triangles én directionele voxels. Gedeelde branchposes, bron-ID’s en geprojecteerde foutselectie blijven leidend. Een impostor kan later een gemeten HLOD-optie zijn; een algemeen billboardpad vervangt deze richting niet. |
| Visibility en shading | Bouw voort op onze visibility buffer, voeg conservatieve HZB/compaction toe en profileer werk per materiaal. Variable-rate shading pas invoeren wanneer dunne vegetatie en motion-history aantoonbaar behouden blijven. |
| Direct licht | Behoud precieze nabije schaduwen, test gecachte world-space foliagepages en gebruik de terreinhiërarchie voor lange terreinzichtbaarheid. Cachevaliditeit en animatiekosten expliciet begrenzen. Screen-space contacts kunnen aanvullen, maar offscreen occluders vereisen world-data. |
| Indirect licht | Ga door met een begrensde surface-/radiance-cache en echte materiaalrespons. Statische broninformatie mag vooraf verwerkt worden; zon, lampen, gewijzigde geometrie en geldigheid moeten correct bijwerken. Geen volledige Lumen-reconstructie vereist voor de eerste bruikbare bounce. |

**Correcties op het Decima-overzicht.** Procedureel plaatsen kost geen *nul MB*: regels, bronassets, habitatdata, werkbuffers, zichtbaarheid en eventuele persistente wijzigingen nemen geheugen en rekentijd in. Het vermijden van een volledige opgeslagen positielijst elimineert die kosten niet. Er is geen aangeleverde gelijkwaardige benchmark die “Decima altijd sneller dan UE5” onderbouwt. De geciteerde talks zijn titel- en jaarspecifiek; zij bevestigen niet de volledige huidige implementatie van Death Stranding 2 of toekomstige Decima-games.

De precieze claims over octahedral impostors met MSAA-A2C, een 3D-windveld zonder skeletons, hoogtevelden voor boomschaduwen op tientallen kilometers en de genoemde foliage-BRDF zijn hier **niet geverifieerd**. Guerrilla’s AA-publicatie uit 2017 beschrijft juist TAA en checkerboard-resolve; gebruik “A2C is hun algemene oplossing” daarom niet als ontwerpfeit. [Lighting and AA](https://www.guerrilla-games.com/read/decima-engine-advances-in-lighting-and-aa).

Relightable SH-probes zijn een bruikbare ontwerpoptie, maar de specifieke beschreven indirect-lightingmethode is met deze bronnen niet aan huidig Decima toe te schrijven. Een primaire uitwerking staat bij **Sucker Punch’s Ghost of Tsushima**: vooraf bepaalde sky visibility, runtime SH-relighting en benaderde bounce. Dat is geen bewijs van onbeperkt dynamisch lichttransport; voor onze wijzigende procedurele wereld blijven invalidatie en echte occluders vereist. [Patry, SIGGRAPH 2021](https://advances.realtimerendering.com/s2021/jpatry_advances2021/index.html).

## Mijlpalen en afhankelijkheden

| Volgorde | Levering | Afhankelijkheid | Acceptatie vóór promotie |
| --- | --- | --- | --- |
| M0 — reproduceerbare baseline | Vaste bos-, rots-, gras- en nachtsequenties met camera, zon, windtijd, seed en instellingen. | Bestaande F12/replay. | Bron/commit, executable-hash, GPU/driver, interne resolutie en alle flags vastgelegd; dezelfde wereld bij iedere variant. |
| M1 — stabiele directe zichtbaarheid **actief** | Gedeelde terreinzichbaarheid plus source-alpha-schaduwpagina’s voor bos. | M0; werkelijke geometrie en begrensde pagepool. | Zonlicht op dezelfde wereldplek blijft gelijk bij yaw, naderen en vier views. Lichte toppen én donkere onderkanten blijven herkenbaar; gaten blijven open. Cachehit, sun-edit, static-edit, eviction en overflow getest. Restpose-afwijking ver weg expliciet gemeten. |
| M2 — één toegepaste vegetatieslice | Gedeelde conifer- en grasassemblies met compacte parts, grove hiërarchie, verre representatie, materiaal en branchwind; boomrasterwerk heeft prioriteit. | M0; bestaande assembly/voxel/part-proeven, M1 voor schaduwvergelijking. | Eén speelbare strook bij 5/40/70/100 m en meerdere hoeken: roots, soort, kleur, openruimte en lichtresponse blijven gelijk. Geprojecteerde fout én optische fout begrenzen de keuze. Native en DLSS motion-history, respawn en recycled IDs eerst correct; geen stille native-fallback. Rasterwerk daalt bij gelijkwaardige coverage. |
| M3 — schaalbare GPU-selectie | Conservatieve HZB, instance/branch-compaction, animatie-LOD en gemeten raster/volume-keuze. | M2 heeft een betrouwbaar bronorakel. | Cameracuts en disocclusion missen niets; wind komt niet buiten bounds; vier views delen poses maar houden eigen zichtbaarheid. Extra selectie is goedkoper dan het vermeden werk. Geen volledig scherm raymarch voor een handvol verre pollen. |
| M4 — eerste echte live bounce | Kleine terrein-surface-cache met dezelfde materiaalcode, begrensde GPU-gather en aparte diffuse bounce. | M1 plus volledige of conservatief onbekende objectvisibility; ontwerp in [bounce-plan](live-terrain-bounce-plan.md). | Rood oppervlak geeft rood teruggekaatst licht; offscreen blocker stopt het; lege/onbekende/ingegraven probes verzinnen geen licht. Geen dubbele heuristische bounce. Direct geblokkeerde ontvanger kan wél indirect licht van een zichtbaar verlicht oppervlak ontvangen. Eerst beperkte speelzone, daarna bos. |
| M5 — wereldresidency en GPU-plaatsing | Compact terrein/habitat/seed → deterministische plant-ID’s; gezamenlijke residency voor vier spelers; later branch/voxelpages en HLOD. | M2/M3; gevalideerde bron- en schaduwidentiteiten. | Geen veranderende populatie bij camerabeweging; gelijke roots/hoogte/kleur/wind in iedere representatie. Streaming en eviction behouden physics en lichtgeldigheid; capaciteit/IO/VRAM begrensd bij uiteengaande spelers. |
| M6 — atmosfeer en brede toepassing | Zoncyclus, schaduwende lokale mist, lampen en uitbreiding naar meer boom-/struikfamilies. | M1/M4/M5; bruikbare tijd- en updatebudgetten. | Sunrise/noon/sunset/nacht én bewegende lampen zonder halo, plotseling donkerder worden, opgeblazen ambient of trails. Vervolgens langere routes en laad-/herstartregressies. |

M2 en het materiaalwerk voor M4 kunnen parallel. Een kleine GPU-plaatsingsproef uit M5 kan daarnaast dezelfde world-cell-ID’s en roots als de CPU-bron bewijzen; volledige residency volgt na M2/M3. Opschalen naar een volledig bos volgt pas nadat de kleine slice optisch en temporeel klopt. Een ontbrekende surface cache wordt niet opgelost door meer probes; onjuiste foliagecoverage niet door meer bomen.

## Budgetten en meetcontract

120 FPS betekent **8,33 ms per volledig frame**. Vier views delen één 4K-outputvlak, maar culling, shading en verschillende zichtbare delen kosten nog steeds extra werk. Een enkele GPU-pass of stilstaand beeld bewijst dit doel niet. Epic noemt voor een Lumen-kwaliteitsniveau circa 8 ms voor GI/reflections alleen op 1080p intern met TSR; dat budget past niet vanzelf in ons hele 120-FPS-frame. [Lumen-doelstellingen](https://dev.epicgames.com/documentation/en-us/unreal-engine/lumen-technical-details-in-unreal-engine).

| Budget | Vastgelegd of voorlopig |
| --- | --- |
| Frame | Doel ≤8,33 ms; rapporteer gemiddelde, p95, p99 en hitches op een vaste rijroute. CPU-mainthread en GPU apart, plus volledige present-to-present tijd. Doel nog onbewezen. |
| Resolutie | Native 3840×2160 en DLSS met exacte input/output apart meten; frame generation telt niet als echte sim/render-FPS. Eén en vier views, inclusief vier gescheiden bosgebieden. |
| Schaduwpagina’s | Standaard pagebreedte 64 m; gecontroleerde bosreplay gebruikt expliciet 256 m. Beide: 512² depth per page, maximaal 256 pages, 8 page-updates per frame en ≤256 MiB depthpayload; metadata/transients apart. Grotere pages verruilen ruimtelijke resolutie voor dekking. Nog geen geaccepteerde route-/vier-viewtijdmeting. |
| Vegetatieproef | Maximaal 256 volume-owners/partnodes, maximaal 64 branches per assembly; toegepaste patch ≤32 graspollen + 4 jonge coniferen. Dit is een proeflimiet, geen wereldbudget. |
| Radianceproef | Maximaal 2.048 probes, 96 richtingen, 4 probe-updates per CPU-call; GPU-cache circa 3,38 MiB inclusief keys. Productieplanning moet ook traversalkosten, surface-atlas en lookups begrenzen. |
| Residency | Huidig standaardgras: 70k gedeelde slots single / 115k split; extended is expliciet een grotere proef. Iedere toekomstige cache rapporteert misses, evictions, clipped work en peak bytes. |
| Iteratie | Meet stagegeneratie, visuals/vegetatie, GPU-upload en eerste zichtbare frame afzonderlijk. Geen winstclaim door gameplay te bevriezen of werk buiten de timer te verplaatsen. |

Voorgestelde optische gate voor M2: maximaal 5% absolute coveragefout in vastgelegde masks en maximaal 1 pixel ruimtelijke representatiefout, plus afzonderlijke directe/indirecte kleur- en silhouetbeoordeling. Deze getallen zijn **acceptatiedoelen**, geen reeds behaalde eigenschappen. Temporaliteit en losse dunne gaten blijven aanvullende gates.

## Bewijs en open regressies

- **Directe bosbelichting blijft open:** [distant-tree-lighting.md](distant-tree-lighting.md), `C:/dev/perf/warp-sideways-20261024/r24-forest/`. Referentie toont dat de grove canopy zowel lichte kroonpunten verduistert als donkere structuur uitwist. Compact-density/source-bounds en één grote far-shadow-map zijn afgewezen; `KERN_FAR_SUN_SHADOW` blijft uit. Nieuwe pagina’s vervangen die acceptatie niet automatisch.
- **Paged-shadow CPU-controles PASS:** na de smallere grass-layout renderer 231 PASS / 1 ignored, inclusief pagebreedtetest; capture 77 PASS / 1 ignored; vier gerichte CPU-tests van de pagefixture PASS.
- **Paged-shadow GPU-fixtures PASS:** de [64 m-proef](C:/dev/perf/warp-sideways-20261024/r25-world-visibility/paged-foliage-fixture-b/report.json) en de [256 m-proef](C:/dev/perf/warp-sideways-20261024/r25-world-visibility/paged-foliage-fixture-256/report.json) bevestigen source-alpha/depth, open gaten en gedeelde vier-viewcache. De 256 m-proef begint met een koude cache en zestien aangevraagde pages: met acht updates per frame ontbreken er acht na het eerste frame, nul na het tweede; volgende cachehits schrijven geen pages. Dit is bewijs uit gecontroleerde scenes, geen hele-boskwaliteit- of 4K/120-acceptatie.
- **Echte bosreplay rendert na layoutfix:** de eerdere `noon-paged-a` mislukte vóór de beelden door 17 storage buffers bij een limiet van 16. [noon-paged-256/report.json](C:/dev/perf/warp-sideways-20261024/r25-world-visibility/noon-paged-256/report.json) rendert wel: 151 gevraagde/residente pages, nul ontbrekend; debug 25 bevat geen magenta. De native 1920×1080-reconstructie houdt wereld, zon en wind vast en bevat geen auto/HUD; dit is niet het oorspronkelijke DLSS-frame.
- **Gerichte bronschaduwvergelijking:** op pixel `(1152,162)` verandert RGB van `14/16/5` naar `115/120/91`; de onafhankelijke nabij-bronschaduw, gericht op datzelfde wereldpunt, geeft `118/124/94` in [noon-source-oracle-b](C:/dev/perf/warp-sideways-20261024/r25-world-visibility/noon-source-oracle-b/report.json). Nabij pixel `(1459,205)` verandert hoogstens circa één byte. Pixel `(537,183)` blijft geblokkeerd in zowel pages als bronreferentie `noon-source-oracle-a`. Dit ondersteunt een gecontroleerde verbetering van zichtbaarheid, geen algemene helderheidsverhoging. Hele-bosoptiek en 4K/vier-view/120 FPS zijn hiermee niet bewezen.
- **Sunset-approach zonder ontbrekende pages:** [rocks-paged-256/report.json](C:/dev/perf/warp-sideways-20261024/r25-world-visibility/rocks-paged-256/report.json) heeft bij 0/20/40/60 m naderen steeds 154 gevraagde/residente pages, nul ontbrekende pages en nul nieuwe updates in de gemeten frames na opwarming. Dit valideert dekking en cachehergebruik bij die vaste wereld/lichttoestand; het meet geen gameplay-FPS of dynamische windschaduwkwaliteit.
- **Terreinvisibility opt-in gevalideerd:** `r25-world-visibility/terrain-gpu-final`, `world-sun-fixture`, `rocks-candidate` en `rocks-comparison.json` onder dezelfde perf-root. CPU/GPU-querypariteit, approach/orbit en vier views zijn getest; ontbrekende mesh-/foliagevisibility blijft afzonderlijk werk.
- **Radiance GPU-proef PASS:** [report.json](C:/dev/perf/warp-sideways-20261024/r25-world-visibility/radiance-cache-gpu/report.json): nul CPU/GPU-mismatches in negen gevallen, lege cache en identieke vierpanelen-uitvoer. Grootste HDR-afwijking circa `2,9e-6`. Dit bewijst sampling in analytische scenes, niet gameplay-GI of routeprestaties.
- **Procedural slice toegepast, kwaliteit nog open:** `r20/grass-stage-auto.png.json` onder dezelfde perf-root; bron/volume/posefixtures in `r16/procedural-foliage-a`. Voxelbeelden waren te zacht en duur. Een geometry-pass, correcte pose of subpixelcel alleen bewijst geen bewaarde optiek.
- **Transmissie-interval hersteld:** [transmittance_tests.rs](../crates/render/src/transmittance_tests.rs), drie CPU/WGSL-regressies PASS. Een 50 m verzoek integreert nu 49,5 m vanaf de bestaande 0,5 m start. Het 12-samplepad blijft een beperkte benadering, geen volledig zichtbaarheidscertificaat of bosvormfix.

Bij iedere update: wijzig eerst de statusrij, voeg het concrete bewijs toe, noteer de volgende blokkade en pas daarna de geaccepteerde speelinstelling aan. Een geslaagde fixture promoveert niet stilzwijgend tot standaard. README/HANDOFF verwijzen naar dit plan; tegenstrijdige oudere experimentbeschrijvingen zijn geen acceptatiebesluit.

## Gemeten prioriteit voor gedeelde vegetatie

**Besluit na r28: eerst het geometriewerk van bomen structureel verlagen, via een gedeeld assembly-, voxel- en branchwindpad voor bomen én gras.** Alleen het bestaande kaartgras verder optimaliseren raakt de grootste gemeten kosten niet. De geaccepteerde grasdiepteprepass blijft behouden: de dedicated graspass ging eerder van circa 9,89 naar 1,02 ms zonder minder pollen.

De gecontroleerde vier-spelermatrix gebruikt dezelfde 3840×2160-output, DLSS Quality, wereld, camera’s, licht en spelersposes. Er zijn 240 frames per run en 179 gekoppelde GPU-metingen na uitsluiting van warmup en diagnostische captures. Alleen kleurgeometrie van één categorie verdwijnt; schaduwcasters, GI-bezetting, physics en effecten blijven staan. [Volledig rapport](C:/dev/perf/warp-sideways-20261024/r28-categories/matrix/comparison/report.json).

| Meting | Uitkomst | Betekenis |
| --- | --- | --- |
| GPU covered interval, mediaan | Baseline 35,52 ms; bomenkleur uit 16,95 ms | Bomen zijn de eerste geometrieprioriteit; bomen verwijderen is geen bruikbare optimalisatie. |
| Gekoppelde marginale daling bij bomenkleur uit | Raster 14,24 ms; cull 2,52 ms; shade 1,47 ms | Vooral selectie en rasterisatie veranderen. Deze verschillen zijn niet optelbare eigen kosten van categorieën. |
| Gekoppelde marginale daling bij graskleur uit | Volledig covered interval 1,87 ms | Gras blijft onderdeel van de nieuwe gedeelde architectuur, maar is nu niet de grootste tijdpost. |
| Bomen in captureframe 240, som over vier views | 1.948.699 clusters, **123.302.820 geselecteerde driehoeken**, **511.958.376 aangeboden vertexslots** | Hoofdvisibilitypad, geen schaduwpasses. De vertexslots omvatten bucketpadding; dit zijn geen unieke opgeslagen vertices en geen gemiddelde over de hele route. |

De tellers komen uit de [vastgelegde capture](C:/dev/perf/warp-sideways-20261024/r28-categories/matrix/baseline/captures/frame-000240.json). Ook zonder bomen blijft het GPU-interval boven 8,33 ms. Het weglaten verandert occlusie en de zichtbare ondergrond; daarom gebruiken we dit als prioriteitsbewijs, niet als voorspelde winst of categorie-FPS.

### Wat we uit Nanite Foliage overnemen

Dit zijn ontwerpprincipes voor een eigen implementatie. Epics huidige documentatie markeert Nanite Foliage als experimenteel; haar demoresultaten zijn geen budgetgarantie voor Warp. CDPR bevestigt specifiek een PS5-technologiedemo op 60 FPS. [CDPR, 3 juni 2025](https://www.cdprojekt.com/en/media/news/cd-projekt-red-and-epic-games-present-the-witcher-4-unreal-engine-5-tech-demo-at-the-state-of-unreal-2025/).

| Primair beschreven principe | Consequentie voor Warp |
| --- | --- |
| **Assemblies:** gedeelde onderdelen; verre part-roots worden gezamenlijk vereenvoudigd, uiteindelijk tot één cluster. Transforms worden bij hiërarchietraversal bepaald. [Epic Assemblies](https://dev.epicgames.com/documentation/unreal-engine/nanite-assemblies). | Niet één blijvende scene-instance/rootcluster per naald. Eén plant bezit een compacte onderdelenhiërarchie met gezamenlijke grove representaties. |
| **Voxelkeuze:** representatiefout beslist tussen triangles en voxels; voxelclusters gebruiken aparte bins en front-to-back rasterisatie. Normalen zijn een verdeling. [Epic Foliage](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage). | Geen vaste afstandsring of groen gemiddelde; behoud openruimte, kleur en richtingsrespons. Begrens brickwerk vóór shading. |
| **Skinning:** bone-matrices leveren strakkere bounds; animatie stopt bij kleine schermgrootte, mede voor schaduwkosten. [Epic Foliage](https://dev.epicgames.com/documentation/en-us/unreal-engine/nanite-foliage). | Bereken branchposes eenmaal voor alle views; kies animatiedetail op hun gezamenlijke behoefte. Huidige/vorige poses, normals en schaduwen moeten overeenkomen. |
| **Procedurele authoring:** PVE scheidt groei/skeleton, meshing, foliage-distributie en export; groeistappen zijn niet gelijk aan kalenderjaren. [Epic PVE](https://dev.epicgames.com/documentation/unreal-engine/procedural-vegetation-editor-pve-in-unreal-engine). | Soortrecepten en groeistadia produceren gecachte assets; geen botanische simulatie van ieder boompje tijdens ieder frame. |

Hier betekent **virtual instancing** het voorgestelde compacte, op aanvraag geëvalueerde onderdelenmodel. Het is niet hetzelfde als onze huidige retained instancebuffer. Epics beschreven assemblies ondersteunen momenteel één instancinglaag; skeletal parts bewegen daar rigide in hun bind pose en zijn niet willekeurig geneste, afzonderlijk geanimeerde skeletons. We nemen geen onbewezen claim over een onbeperkte hiërarchie of gratis wind over. [Epic beperkingen](https://dev.epicgames.com/documentation/unreal-engine/nanite-assemblies).

### Concrete gaten tussen broncode en doel

| Gat | Actuele bron en beperking | Eerstvolgende levering |
| --- | --- | --- |
| Renderbare assemblies zonder expansie | r30 deelt unieke primitive-meshes en lokale plaatsingen via `CookedPlant`/`SceneVegetation`. Bronexpansie gebeurt alleen bij voxel-cooking en het onafhankelijke testorakel. De GPU past branchposes toe op members. | Voeg gezamenlijke grove traversal/aggregatie toe; één instance per blad bespaart geometriegeheugen maar kan cull- en rasterwerk verhogen. [Gedeelde bron](unified-vegetation.md). |
| Veel soorten en echte groeivormen | Speelpad: drie vervormde evergreen-silhouetten, geen drie botanisch afzonderlijke soorten. Prototype: alleen `GrassTuft` en `ConiferTwig`; groei schaalt rustafmetingen. | Herbruikbare blade/needle/leaf/wood-primitives plus datarecepten voor vertakking, bladstand, leeftijd en materiaal. Eerst een conifertak en graspol, daarna volledige boom en struik; geen unieke mesh per wereldplant. [Spelprofiel](../../games/sideways/sim/src/vegetation.rs). |
| Schaalbaar verre-detailpad | `voxel_forest` is een begrensde compute-integrator met actieve tiles. Maximaal 256 owners/poses; bronoptiek is expliciet `Unvalidated`. r30 meet nog te hoge integratiekosten bij slechts 71 geraakte pixels. Pixelcompactie was circa 24% trager en is afgewezen. | Zware integrator en cluster/brick-traversal gericht onderzoeken; gezamenlijke grove hiërarchie toevoegen. Niet opnieuw aannemen dat minder gestarte pixels vanzelf sneller zijn. De kleine multi-volume raymarch niet rechtstreeks over honderdduizenden bomen uitrollen. [Bewijs](unified-vegetation.md), [selector](../crates/render/src/foliage_representation.rs). |
| Branchwind met reconstructie | Part-wind deelt huidige/vorige poses, maar `Renderer::set_upscale` weigert DLSS bij voxel/branch-foliage. Productiebomen gebruiken nog instance-shear. | Stabiele plant/branch/generation-ID’s, vorige pose en gekozen representatie naar motion/depth doorgeven; gewisselde of gerecyclede geschiedenis afwijzen. Normals inverse-transpose, schaduwpose en windbounds gelijk houden. [Part-wind](../crates/render/src/part_wind.rs), [frame](../crates/render/src/frame.rs). |
| Visibility en wereldbudget | Frustum/CLOD en actuele meadow-/r38-boom-Hi-Z aanwezig; boomtemporaliteit heeft nog een1-pixelfout. Geen algemene geometry-page-streaming of GPU-boomhabitatplaatsing; meadow genereert roots al op de GPU. | Temporele occlusie verklaren en gezamenlijke world-cell-residency toevoegen; vier views delen assets/poses, maar hebben eigen zichtbaarheid. Alle extra selectiekosten meenemen. |

### Eerstvolgende patch en acceptatie

**r30 heeft de niet-geëxpandeerde bron, gedeelde GPU-branchposes en een toegepaste gemengde strook. M2 blijft open.** De eerstvolgende gates zijn de gemeten voxel-kosten, gezamenlijke grove selectie, mooiere recepten en optische equivalentie. Het scenario `games/sideways/scenarios/unified-vegetation.toml` legt de strook en camera's vast. Rustgegevens worden per recept/groeistadium gedeeld; world-cell-ID, materiaalvariant en windfase blijven door alle representaties gelijk. Volledige cooking-cache en wereldresidency volgen nog.

Promotie vereist dezelfde camera’s, zon en wind op 5/40/70/100 m, silhouetten en interne gaten, schaduw/normalen en kleurbehoud, gevolgd door beweging in native én DLSS en vier uiteenlopende views. Iedere verandering rapporteert geselecteerde clusters/driehoeken, padded vertices, branchposes, volume-steps, bytes/residency en volledige GPU-intervaltijd. Soortenrijkdom mag niet verborgen lineair renderwerk per onderdeel toevoegen. Alleen sneller bouwen of meer geheugen delen is onvoldoende; de gemeten boomrasterkosten moeten dalen bij gelijkwaardig beeld. Nieuwe kwaliteit en prestaties moeten zichtbaar zijn in het spel voordat deze richting als afgerond geldt.
