# Software raster voor pixelgrote clusters (r53)

Status: **geïmplementeerd achter `KERN_SOFTWARE_RASTER=<px>`, standaard uit,
alleen gecontroleerd op lavapipe.** Bouwt voort op de opake kronen en
voxelniveaus van [r51](opaque-crowns-r51.md).

## Waarom

Opake naaldgeometrie en voxelblobs maken veel driehoeken van een pixel of
kleiner. Een hardware-rasterizer schaduwt in blokken van 2x2 pixels en betaalt
per driehoek vaste set-upkosten, dus bij zulke driehoeken gaat het meeste werk
verloren. Nanite rastert die clusters daarom in een compute shader met 64-bit
atomics; Epic mat kaarten 3 tot 4 keer trager dan geometrie in dat pad.

## Wat het doet

- `cull.wgsl` (`software_cluster`): in camera-kleurviews gaan clusters naar
  een aparte lijst als hun hele cullbol hooguit `<px>` pixels breed
  projecteert, ruim voor het nabije vlak ligt, het materiaal ondoorzichtig is
  (geen alpha test, gras of blend) en de instantie geen gras of
  voxelvervanging is. Cull en scatter rekenen dit identiek uit; de lijst staat
  achter de vier buckets in `draw_map`, met dispatch en aantal in `args[20..25]`.
- `software_raster.wgsl`: één workgroup per cluster, één thread per driehoek,
  dezelfde vertexpositie en wind als `raster.wgsl`. Elk geraakt pixelcentrum
  doet één `atomicMax` op (reversed-Z-diepte, zichtbaarheids-id) in een
  64-bit buffer per view. Tweezijdig zoals de hardware-pipeline.
- `software_merge.wgsl`: een schermvullende driehoek schrijft per pixel id en
  diepte naar de gewone zichtbaarheids- en dieptetargets met de gewone
  Greater-dieptetoets. Shading, bewegingsvectoren, picking en de
  boomocclusiepiramide zien daardoor één zichtbaarheidsbuffer, en de twee
  boomocclusiefasen werken ook.
- Vereist `SHADER_INT64` en `SHADER_INT64_ATOMIC_MIN_MAX` (Vulkan en DX12 op
  de 5070 Ti, ook lavapipe). Zonder die features blijft alles hardware.

## Controle

- Shadervalidatie en de gedeelde `args`-indeling zijn getest; 504 renderer-
  en 186 gametests slagen.
- Lavapipe, `sideways shot`, 960x540, opake kronen, zonder verre bossen, één
  frame zonder TAA: met 32 px gaan **244.007 van 395.574 clusters** door de
  software raster. Het beeld wijkt op 163 van 518.400 pixels meer dan 8/255
  af van hardware alleen (gemiddeld 0,013/255): randregels en gelijke
  diepten.
- Lavapipe-tijden zeggen niets over een GPU, maar ter indicatie: hardware
  raster 10,7 s naar 5,1 s plus 1,6 s software raster.

## Nog niet

- GPU-tijd: `opaque-crowns.toml` met `opaque-crowns.opaque.json` tegen
  `opaque-crowns.opaque-software.json` op de 5070 Ti, eventueel ook 16 en 64 px.
- Schaduwkaarten: daar ging op lavapipe de meeste tijd heen (250 miljoen
  schaduwtriangles). Diepte-alleen heeft geen 64-bit atomics nodig (32-bit
  atomicMin op diepte), maar de dieptebias van de hardwarepipeline moet
  worden nagebootst.
- Alpha-geteste kaarten gaan niet door dit pad.
