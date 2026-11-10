# Spell effects

What the spells look like and how that is built. The code is
`app/src/fx.rs` (particles, bolts, impacts, crashes, sprays, beams, fields)
and `app/src/fx.wgsl` (the procedural look of each particle cell and of the
fields on the ground). The design these follow is the Claude doc
"Spellstorm spell effects: design".

## Layers

Every effect is built from the same layers, and each element fills them its
own way:

| Layer | What it is | Where in `fx.rs` |
| --- | --- | --- |
| Core | The brightest, smallest part, near white | per bolt in `Fx::bolt` |
| Body | The element's silhouette: flame tongues, an ice spike, a water orb, a black disc | `Fx::bolt`, `style` |
| Secondary | Small fast pieces stretched along their flight: embers, glints and shards, drops, sparks, grit, drips, motes, wind lines | `bits(e, k)` |
| Residue | What hangs after: smoke, cold mist that sinks, toxic gas, spray mist, dust | `haze(e, k)` |
| Light | A local light at the core and on hits | `Fx::light`, `light_pop` |
| Ground | A mark that fades after a hit: scorch, frost bloom, wet patch, slick | `Fx::scar` (the field looks of `fx.wgsl` `ground`) |

A staff bolt carries core and body at its head and sheds its secondary and
residue behind, so it draws its own trail. An impact (`Fx::impact`) is a
flash, the secondary thrown back off the spot, a puff of residue and a short
ground mark. A crash (`Fx::crash`, novas and thrown spells) adds the
element's set piece (blast, smoke volumes, shards flung out flat), then the
secondary over the whole splash and a lasting mark.

## Elements

| Element | Body | Secondary | Residue | Ground |
| --- | --- | --- | --- | --- |
| Fire | flame tongues (`flame`), a hot heart | embers that rise and cool | dark smoke | scorch with embers winking out |
| Frost, Ice | a faceted ice spike flying point first (`Shard`) | glints that twinkle (`Glint`), shards | cold mist that sinks | frost bloom |
| Water | a clear orb with a white rim (`Bubble`) | drops that stretch as they fall | fine mist | wet patch with rings |
| Lightning, Storm | a flickering spark with small arcs | fast sparks | (the arc's afterimage) | |
| Earth, Mud | a lit rock or gob of mud | grit and chips | dust lit by the lights round it | mud splat |
| Magma | molten rock (`MOLTEN`) | burning drips and embers | black smoke | molten patch |
| Poison | a gooey acid blob with bubbles | drips, bubbles that pop (`Bubble`) | low green gas | slick |
| Void | a black disc inside a violet rim | motes spiralling in | a dark bloom | a hole with a violet swirl |
| Air | a vortex ring across its flight | long wind lines | dust | |

## Particle cells

`Cell` in `fx.rs` lists the atlas cells; each has a baked texel and may have
a procedural look in `fx.wgsl` (`cell_look`, effect 1 + the cell's index).
`Shard`, `Glint`, `Ember` and `Bubble` are drawn by `fx.wgsl`. An emitter
with `stretch` > 0 draws each particle as a streak along its velocity on
screen (length grows with speed); `Ember` and `Shard` are made for that, and
the flamethrower's flames use it to read as tongues.

## Colour and brightness

Effect sprites are added to the HDR image before AgX. Two things follow:

- Colours written in `fx.wgsl` are linear. A colour picked by eye is sRGB
  and must go through `lin()`, or a yellow shows as cream.
- A bright saturated red or orange desaturates in AgX into salmon pink.
  Fire keeps brightness tied to heat (dim gas red, bright gas yellow) and
  leans its oranges towards yellow; flame particles glow at 0.8.

## GPU particles

With `KERN_GPU_PARTICLES=1` a full-strength crash adds dense extras on the
GPU (`Fx::gpu_extra`): embers off fire and magma, glitter off frost and ice,
sparks off lightning, motes round a void. Effects look whole without them.

## Looking at them headless

```sh
spellstorm shot --out bolt.png --bots 0 --humans 1 --ticks 600 --bolts frost
spellstorm shot --out spray.png --bots 0 --humans 1 --ticks 600 --cast-at 560 --cast fire,fire
spellstorm shot --out nova.png --bots 0 --humans 1 --ticks 620 --lua-at 600 \
  --lua 'local w = g:wizards()[1]; local p = g:pos(w); g:nova(vector.create(p.x, 0, p.z - 4), 3, "frost")'
```

`--bolts` fires the first wizard's staff bolts of that queue every 8 ticks
over the last 48; `--cast` with `--cast-at` starts a spray, beam, strike or
throw at that tick.
