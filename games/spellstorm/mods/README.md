# Spellstorm's spell rules in Luau

Every `*.luau` file under `mods/` is a module. It reloads when saved, while
the game runs: a file that does not load keeps the version before it, and a
hook that fails at runtime goes back to the version before your edit (or
stops until the file changes). Modules run inside the 60 Hz sim tick in path
order, so a seed with the same mods replays exactly. What things do stays
Rust: how monsters move and attack, how a level is laid out, how each
shape flies and lands. How strong, fast and tough they are, and what each
level is made of, is Luau.

Start the game with `spellstorm play --live` to edit through the live tools
(`write_file mods/...`, `run_luau`, `command mods`), or just save the files.
In the console: `mods` lists the modules and their errors, `lua CODE` runs a
line against the world as `g`.

Sandbox: no `os`, `io`, `require` or `math.random` (use `g:random()`). A
call gets a step budget; `state` is a per-module table that survives
reloads; `print` goes to the game's log.

## What a module can return

```luau
return {
  compose = function(slots, form, mods) ... end,  -- the grammar (one module: the last in path order)
  effects = { fire = { dmg = 7, burn = 1 }, ... }, -- per element; later modules override per element
  tomes = { { name = "...", text = "...", mods = { fire = 1.3 } }, ... },  -- replaces the list
  monsters = { { key = "ghoul", name = "Ghoul", hp = 40, ... }, ... },     -- replaces the list
  biomes = { { id = "chapel", name = "...", enemies = { { "ghoul", 3 } }, ... }, ... },  -- replaces the list
  cast = function(g, e) ... end,  -- hooks, every module that has them, in path order
  hit = function(g, e) ... end,
  tick = function(g) ... end,
}
```

- `base/monsters.luau` holds every monster kind, `base/biomes.luau` the
  six levels in run order; their headers list every field. A changed
  value applies at once (a level's layout at the next level). Adding,
  removing or reordering kinds or biomes waits for the next start, because
  monsters and levels alive point into the lists. A biome must name
  monsters that exist and a boss with `boss = true`, or the whole change
  is refused and the lists before stay.
- `base/spells.luau` holds the grammar and the element effects, a port of
  Godot's grammar.gd and element.gd. `compose(slots, form, mods)` gets the
  queue as element names (`{"fire", "earth"}`), the form (`"cast"`,
  `"nova"`, `"plant"`, `"enchant"`) and the caster's tomes, and returns
  `{ shape, name, primary, payload, riders, key, n }`. Shapes: staff, stomp,
  fizzle, spray, beam, strike, projectile, nova, trap, sentry, enchant,
  overload. `n` holds the numbers the shape reads (range, angle, duration,
  width, radius, pieces, spread, speed, charge_time, splash, pierce, field,
  field_time, stun, interval) and `rate`, how hard it hits. The game fits
  `rate` so every spell is worth about the same; `n.fit = false` keeps yours.
- `base/tomes.luau` lists the tomes; the comment at its top names the mods.
- `base/knobs.luau` holds the balance knobs (walking speed, damage and
  health scales, wave spacing, the mist walls' rules) in a `-- @knobs`
  block, one number per line. The game reads it, not the module host: a
  saved value applies on the next frame, and `set_knob` rewrites the line.
- Effects: `dmg`, `poison`, `push`, `burn`, `chill`, `wet`, `shock`, `slow`
  per unit of the element. The fit counts them too, so more damage on an
  element makes its spells hit a little less often; the `spell_damage` knob
  scales every wizard's damage after the fit.
- Base names every module sees: `ELEMENTS.fire = { index, name, kind, parts,
  color }` (kind: solid, energy, spirit, flow), `QUEUE_SIZE`, `ENCHANT_TIME`.

## Hooks

- `cast(g, e)`: a wizard cast (mana paid; a spell that winds up asks when it
  is let go). `e = { caster, seat, pos, aim, spell }`, `spell` as compose
  returns it. Return `false` to skip the built-in effect (and do your own),
  a spell table to cast that instead (same timing: let go spells stay ones
  that wind up), or nothing to keep it.
- `hit(g, e)`: before a spell lands. `e = { target, by, strength, elements }`
  (`elements.fire = units`). Return the new strength, or nothing.
- `tick(g)`: every tick. Keep it cheap.

## `g`, the live world in a hook

Ids are numbers. Positions are vectors (`vector(x, y, z)`, y up; the way on
runs towards -z).

| Method | What it does |
|---|---|
| `g:time()` | Seconds since the level began. |
| `g:random()` | Uniform in [0, 1), from the run's seed. |
| `g:wizards()` / `g:enemies()` | Ids of every wizard / living monster. |
| `g:near(at, r)` | Living monsters within r metres. |
| `g:pos(id)`, `g:aim(id)` | Where it stands; the way a wizard aims. |
| `g:health(id)`, `g:max_health(id)`, `g:alive(id)`, `g:is_wizard(id)` | |
| `g:hurt(id, amount, element?, by?)` | Hit with `amount` units of an element. |
| `g:nova(at, radius, element, power?, by?)` | A burst of one element. |
| `g:burst(at, color, n?, speed?, size?)` | Sparks (looks only). |
| `g:shake(amount)` | Shake the camera. |
| `g:mana(id)`, `g:set_mana(id, m)` | A wizard's mana. |

## Examples

A fire nova where every boulder lands (`mods/live/boulder_fire.luau`):

```luau
return {
  cast = function(g, e)
    if e.spell.shape == "projectile" and e.spell.primary == "earth" then
      g:nova(e.pos + e.aim * 6, 3, "fire", 1, e.caster)
    end
  end,
}
```

Frost spells hit twice as hard on burning monsters is a `hit` hook; a new
spell name is a change in `base/spells.luau`'s noun tables.
