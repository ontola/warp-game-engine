# Wind uit op afstand, verre bomen als gecachete schaduwcasters (r52)

Status: **geïmplementeerd achter `KERN_WIND_FREEZE`, standaard uit, niet
gemeten op een echte GPU.**

## Waarom

De statische zoncache van [r45](static-sun-cache-r45.md) leverde niets op:
2.641 statische tegen 319.184 dynamische owners, omdat elke boom op elke
afstand in de wind beweegt. Bij zonsopkomst blijven 52,7 miljoen dynamische
schaduwtriangles over. Epic lost hetzelfde op door animatie onder een
schermgrootte uit te zetten, zodat verre bomen in rustpositie staan en hun
virtual shadow map-pagina's geldig blijven.

## Wat het doet

- `wind.wgsl`: bomen (niet gras, niet part wind) verliezen hun zwaai tussen
  `start` en `einde` meter (horizontaal) van de dichtstbijzijnde camera
  (smoothstep) en staan daarbuiten exact stil. Eén straal voor alle views en
  passes, dus hoofdbeeld, schaduw en bewegingsvectoren blijven gelijk. Hooguit
  vier camera's; bij meer views blijft de wind overal aan.
- `static_sun.rs`: bomen die minstens `einde + guard` van elke vastgepinde
  camera staan, zijn statische casters. De pin wordt vernieuwd zodra een
  camera meer dan `guard` (`KERN_SUN_CACHE_GUARD_METRES`) van zijn pinpositie
  is gereden. Zo staat elke statische boom altijd buiten de windstraal (getest:
  `frozen_trees_become_static_only_beyond_the_pinned_radius`). Een nieuwe pin
  wist de statische diepte en tekent die één frame opnieuw.
- De gedeelde 4096²-zonkaart was al één kaart voor alle views; de nabije
  2048²-kaarten per view blijven dynamisch.

Aanzetten (alle drie nodig voor de schaduwwinst):

    KERN_STATIC_SUN_CACHE=1 KERN_SUN_CACHE_GUARD_METRES=32 KERN_WIND_FREEZE=50,70

## Eerste meting (lavapipe, geen GPU-tijd)

`sideways shot`, 960x540, één view, zonder verre bossen, wind 0,3, middag,
laatste van 8 frames:

| | Statisch / dynamisch casters | Statisch / dynamisch schaduwtriangles |
|---|---:|---:|
| Cache + guard | 281 / 32.027 | 65.305 / 12.462.901 |
| + wind uit vanaf 50 tot 70 m | 31.672 / 636 | 11.176.883 / 1.323.672 |

De dynamische schaduwraster per frame daalt dus ongeveer **9,4 keer**. Het
beeld verschilt alleen in bomen buiten 50 m (rustpositie in plaats van een
zwaai op dit tijdstip) en in hun schaduwen op de weg.

## Nog niet gemeten of opgelost

- GPU-tijd: `games/sideways/scenarios/fourplayer-route.toml` met
  `wind-freeze.cache-only.json` en `wind-freeze.on.json` (en zonder
  env-bestand als nulmeting) op de 5070 Ti.
- Elke nieuwe pin (ongeveer elke 32 m rijden) tekent de hele statische kaart
  opnieuw: een piek van ongeveer een volledige zonraster in één frame.
  Pagina's of een over meerdere frames verdeelde hertekening zouden die piek
  wegnemen.
- De zichtbare grens: op 50 tot 70 m is een zwaai van enkele pixels nog te
  zien. 50,70 is een eerste keuze, geen geaccepteerde instelling.
