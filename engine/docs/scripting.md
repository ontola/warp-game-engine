# Scripting and modding with Luau

The `script` crate runs [Luau](https://luau.org) modules for a game. The
engine only provides the host: the sandbox, loading and hot reload, and the
limits. The game decides which hooks exist, what they receive and which
functions scripts may call. Growing Guns is the worked example: its 52
cards and its rule hooks are Luau. The [`warp` executable](warp-cli.md)
goes further: a whole game in Luau, with a game API of its own.

## The host

`script::Host` loads every `*.luau` file under a folder as a module (type
definition files, `*.d.luau`, are skipped). Each module runs in its own
environment and returns a table of exports: hooks (functions) and data.

```rust
use script::{Host, Limits};

let mut host = Host::new(Limits::default(), |lua, api| {
    // The game's names, frozen together with the safe libraries.
    api.raw_set("twice", lua.create_function(|_, x: f64| Ok(x * 2.0))?)
})?;
host.watch("games/my-game/mods");     // load everything, watch from now on

// Every frame or tick:
host.poll();                          // reload what changed on disk
for (module, f) in host.each("tick") {
    host.run(module, "tick", |_| f.call::<()>(()));
}
for line in host.drain_log() { println!("{line}"); }
```

- `Host::new(limits, api)` builds the VM; `api` adds the game's functions,
  constants and tables before everything is frozen read-only.
- `watch(root)` loads every module and remembers the folder; `poll()`
  reloads new and edited files (in path order) and drops deleted ones.
- `add_source(name, src)` adds a module from a string that never reloads,
  for built-in content and tests.
- `has(hook)` and `each(hook)` find the modules that export a function by
  that name; `each` skips broken modules and keeps path order.
- `run(module, what, f)` makes one call with a fresh step budget. A failure
  is logged and turns that module's hooks off until its file changes.
- `generation()` changes on every successful load, reload or removal, so
  the game knows when to re-read exported data such as card tables.
- `console_line(line)` compiles a console line, either an expression
  (`1 + 2`) or statements, in an environment that persists between lines.
  The game binds its live objects into `console()` around the call.
- `drain_log()` returns lines from `print` and from loads and failures.

## The sandbox

The rules keep scripts safe to run inside a deterministic simulation:

- No clock, no files: there is no `os`, `io`, `require`, `loadstring` or
  `getfenv`. The libraries are `math`, `string`, `table`, `utf8`,
  `bit32`, `buffer` and `vector`, plus the basic functions (`pairs`,
  `pcall`, `setmetatable` and so on).
- No `math.random` or `math.randomseed`. The game hands out seeded random
  numbers itself.
- The shared base and the libraries are read-only; a module cannot change
  what another module sees.
- `print` goes to the host's log, not to stdout.

## Limits

`Limits` has two fields, with these defaults:

| Field | Default | Meaning |
| --- | --- | --- |
| `memory` | 64 MiB | Bytes for the whole VM. |
| `steps` | 200,000 | VM interrupts (function calls and loop back-edges) per hook call or load. |

The step budget is counted in VM interrupts, not in time, so a runaway loop
stops at the same point on every machine instead of freezing the game. The
error says "step budget used up (a loop that never ends?)" and that
module's hooks stay off until the file changes.

## Hot reload

Save a file while the game runs and `poll()` reloads it.

- Each module has a `state` table that survives reloads. A running match
  keeps what its scripts remembered while their code is replaced.
- A load error (a syntax error, a module that does not return a table)
  keeps the previous version running and logs the file and line, for
  example `sub/m.luau:3: ...`.
- A runtime error turns that module's hooks off until it changes again.

```luau
-- state survives reloads: this counts how often the file was loaded
state.loads = (state.loads or 0) + 1
return {
    tick = function() end,
}
```

## Worked example: Growing Guns mods

Growing Guns' cards and rules live in `games/growing-guns/mods/`. The
complete API is generated from the Rust side into
[`mods/API.md`](../../games/growing-guns/mods/API.md) and
[`mods/types.d.luau`](../../games/growing-guns/mods/types.d.luau) by
`growing-guns mods-api`; read those for every field and function.

How it is wired (`games/growing-guns/app/src/scripts.rs`):

- The folder is `GG_MODS`, else `games/growing-guns/mods` found above the
  working directory or the executable. Without a folder, the built-in copy
  of `base/cards.luau` is used.
- Modules run on the host inside the 60 Hz sim tick, in path order. The
  same mods and seed replay exactly; clients only load card data for their
  menus.
- The game polls for edits while playing; a saved file is live within
  about a quarter second.

A module returns a table; these keys mean something:

| Key | When it runs |
| --- | --- |
| `cards` | A list of card definitions, merged into the pool. |
| `hit(g, e)` | Before damage lands. Return the damage to deal, or nil to keep it. Modules chain. |
| `kill(g, e)` | A player died (`e.attacker` is nil for lava). |
| `tick(g)` | End of every sim tick. Keep it cheap. |
| `round_start(g)` | A new round's arena and spawns are ready. |

`g` is the live match, valid only during the call that received it.
Players are numbers 1 to `g:players()`. It reads and changes positions,
velocities, health, ammo, weapon stats and cards, and queues damage,
cards and explosions for the start of the next tick (with quotas, such as
32 explosions per tick). `g:random()` is uniform in [0, 1) from the match
seed.

A card mutates the player's `Weapon` (field names from Godot's
`weapon.gd`), either with `apply(w)` or as data with `mul` and `add`:

```luau
-- mods/house/rules.luau: an example mod
return {
    cards = {
        { id = "glass_cannon", name = "GLASS CANNON", desc = "Double damage, half health",
          tags = { "firepower" }, color = Color(1.0, 0.4, 0.6), rarity = "rare",
          apply = function(w)
              w.damage_mult *= 2
              w.max_hp_bonus -= 50
          end },
        { id = "quick_hands", name = "QUICK HANDS", mul = { reload_mult = 0.7 } },
    },

    -- Headshots deal a bit more in this house.
    hit = function(g, e)
        if e.head then return e.damage * 1.25 end
        return nil
    end,

    kill = function(g, e)
        state.kills = (state.kills or 0) + 1
        if e.attacker then g:set_health(e.attacker, g:max_health(e.attacker)) end
    end,
}
```

Weapons are rebuilt from the cards every round, so `apply` must depend
only on `w`. Effects on the match go in a card's `pick(g, p)`, which runs
once when the card is taken.

### The console

The debug console runs Luau against the live match. In a script it is a
`cmd` step:

```json
{ "cmd": ["lua g:give(1, 'explosive')", "lua g:stat(1, 'explosive_radius')", "mods"] }
```

`mods` lists the loaded modules and their errors.
`games/growing-guns/scripts/mods.json` is a headless test of this path.

## In progress

A Luau front end for the procedural synth's patches is being built: sound
patches written as Luau and compiled to `audio::synth::Patch` data. It is
not committed yet. See [audio](audio.md).
