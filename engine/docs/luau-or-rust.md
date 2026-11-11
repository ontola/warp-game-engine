# Luau or Rust: where code goes

Warp is built for fast iteration, and Rust builds are slow. So game code
starts in Luau, where a saved file is live in a quarter of a second without
losing the play session, and moves to Rust when it has to. The engine core
is Rust. Work done for every pixel, every tree or every particle belongs on
the GPU, in neither.

The short rule: **Luau decides, Rust does, the GPU does it for everything
at once.** Luau holds the rules, the numbers and the content: what a card
does, how hard a stage is, which spell a queue of elements makes. Rust holds
the systems those rules steer: movement and collision, physics, generation
algorithms, anything that loops over many things every tick. Luau speaks to
Rust in few, coarse calls: data, events and requests, not one call per
entity per tick.

This page says where each kind of code goes, what a crossing costs, the
patterns the games in this repository use, and how to move code from one
side to the other. The Luau host itself is described in
[scripting](scripting.md), the Luau game API in [the warp
executable](warp-cli.md).

## The three places

| | Luau | Rust | GPU (WGSL) |
| --- | --- | --- | --- |
| Edit to live | ~0.25 s, keeps the session ([hot reload](scripting.md#hot-reload)) | a rebuild and a restart: about 5 s for a game crate, 16 s after an engine edit, then the session is gone ([what a Rust change costs](#what-a-rust-change-costs)) | next frame for a game's `surface.wgsl` ([your own shader](warp-cli.md#your-own-shader)); engine shaders need a Rust rebuild |
| Cost of one step | ~35 ns per loop iteration, ~120 ns per call | ~1 ns | spread over thousands of threads |
| Who may change it | players, modders, designers, an AI over MCP | developers | developers; a game's own surface shader |
| Safety | sandbox, step budget, memory cap: a bad edit cannot crash or hang the game | full access | a bad shader keeps the last good one (game surface only) |
| Types | `types.d.luau`, checked by `warp check` and editors | the compiler | naga at load |

## Put it in Luau when

- **It is a rule or a number someone will tune.** Damage, timings, costs,
  spawn tables, difficulty curves, card and spell definitions, track
  recipes, species parameters. These change many times a day during
  development; each change should not cost a build.
- **It runs on an event, not on every entity.** A hit lands, a player
  dies, a round starts, a card is picked, a wizard casts. A handful of
  calls per tick costs microseconds.
- **It is content.** Cards (Growing Guns), spells and tomes (Spellstorm),
  stages and balance (SpaceRacer), sound patches (`sounds/*.luau`), editor
  pages. Content should be moddable, and Luau is the mod surface.
- **It is glue for a small game.** A whole game of a few hundred entities
  runs fine in Luau: Neon Siege, with 96 enemies, 600 shots, their hits
  and the bursts, takes about half a millisecond a tick.
- **It is a debug helper or a tuning tool.** Console lines, knobs, Pine
  lab's parameter panel. An AI can call it over MCP (`run_luau`).

## Put it in Rust when

- **It loops over many things every tick.** Anything that does work for
  each of more than a few hundred entities, 60 times a second: movement,
  steering, collision, flocking, pathfinding, visibility, physics.
  At ~35 ns per Luau loop iteration and ~120 ns per call into Rust, a loop
  over 2,000 entities that makes three engine calls each costs about a
  millisecond before it does any real work. The same loop in Rust costs
  microseconds.
- **It is an algorithm, not a rule.** Track generation from a recipe,
  terrain height, tree growth, mesh building, spatial queries. Luau
  supplies the recipe; Rust runs it.
- **Correctness must hold for every input.** Invariants, bounds, anything
  online play or replays depend on (beyond the [determinism
  rules](warp-cli.md#determinism) the Luau API already enforces).
- **It touches the engine's insides.** Renderer, scene, physics world,
  audio graph, networking, the asset pipeline. Luau never sees these
  directly; it gets a typed, validated API to them.
- **It has settled and is large.** A 2,000-line system that has not
  changed in weeks gains little from hot reload and a lot from the
  compiler's help when it is refactored.

## Put it on the GPU when

- **It is per pixel, per vertex, per instance in the thousands, or per
  particle.** Shading, culling, vegetation placement and LOD, grass, GPU
  particles, debris. Neither Luau nor a CPU loop in Rust: the CPU only
  sets parameters.
- Luau steers GPU work **as data, uploaded once**: a species' growth rules
  (`plants.grow` in Pine lab), placement rules, colours and curves. A curve
  that Luau defines is sampled into a table or texture by Rust when it
  changes, never evaluated in Luau per tree. With 100,000 trees, even one
  Luau call per tree per frame would cost more than the frame.

## What a crossing costs

Measured with `cargo run --release -p script --example call_cost`, inside
the real `script::Host` (sandbox and step budget on), on a 4-core 2.1 GHz
Xeon cloud VM. A desktop is roughly twice as fast; the ratios hold.

| Operation | ns |
| --- | ---: |
| Empty Luau loop iteration | 35 |
| Luau vector add and scale (`v += d * 0.016`) | 39 |
| Luau arithmetic with `math.sqrt` | 118 |
| Luau function calling a Luau function | 123 |
| Luau calling Rust, numbers in and out | 116 |
| Luau calling Rust, a vector in and out | 125 |
| Luau calling Rust that returns a new table (like `world.raycast`) | 1,780 |
| One Rust call over a list, per element | 80 |
| Rust calling a Luau hook (`Host::run`) | 137 |

What the numbers say:

- **The step budget dominates tight Luau loops.** It is an interrupt at
  every loop iteration and call. Without it (plain mlua) an empty
  iteration is 4 ns instead of 35, and a call to Rust 52 ns instead of
  116. The budget is what makes a runaway loop stop at the same point on
  every machine, so it stays; count with it.
- **A call into Rust costs about as much as a Luau call.** Calling the
  engine is not the problem. Calling it 10,000 times a tick is.
- **Tables made in Rust are expensive.** A call that answers with a fresh
  table (`world.get`, `world.raycast`, `world.overlap`) costs over a
  microsecond. Ask for what you need once per tick and keep it, rather
  than asking per entity per frame. Most of that cost is the memory cap:
  while it is set, mlua guards every allocating call, which makes building
  a table from Rust about four times slower. Rust code that converts
  arguments or results outside a hook (serde `to_value`, a hand-built
  table) should do it inside `Host::unguarded` (or `script::unguarded`
  within a `Host::run` closure), which lifts the cap for the conversion
  only. SpaceRacer's rule calls do this.
- **Vectors are cheap.** Luau's `vector` is a value type: use it for
  positions and velocities instead of `{x, y, z}` tables.
- A rule of thumb for a 60 Hz game: **about 10,000 Luau-side operations
  (calls, table reads in a loop, engine calls) per tick cost a
  millisecond.** Keep a game's Luau under a millisecond or two per tick.

## Patterns

These are in the repository now; copy them.

### Luau returns data, Rust validates and runs it

SpaceRacer's `rules/game.luau` and `rules/stages.luau` return the balance
numbers, the environment for a biome and a recipe for each track. The sim
(`games/spaceracer/sim/src/rules.rs`) reads them through serde into typed
structs, rejects anything outside supported bounds, and runs the track
generator in Rust. Luau never builds geometry; Rust never hard-codes a
balance number.

```rust
// games/spaceracer/sim/src/rules.rs (shortened)
pub fn balance(&mut self) -> Result<Balance, String> {
    let b: Balance = self.call("balance", ())?;
    if b.respawn_seconds < 0.1 || b.race_timeout < 10. /* ... */ {
        return Err("balance values outside supported bounds".into());
    }
    Ok(b)
}
```

Validation at the boundary is what makes it safe to let a modder, a
designer or an AI edit the Luau side freely.

### Re-read data only when the scripts change

`Host::generation()` changes on every load, reload or removal. Read
exported tables (cards, effects, tomes) into Rust structs when it changes,
not every tick. Spellstorm's `Rules::refresh` and Growing Guns' card pool
do this.

### Skip the crossing when nobody listens

Spellstorm keeps one flag per hook (`HIT`, `CAST`, `TICK`), set from
`Host::has` after each load. The sim checks an atomic before it locks the
host, so a game without a `hit` hook pays nothing per hit.

### Cache pure answers

Spellstorm's `compose(slots, form, mods)` is a pure function of its
inputs, so its results are cached by key and the cache is cleared on a
new generation. A spell is composed in Luau once, not on every cast.

### Events and requests, with quotas

Hooks receive events (`hit`, `kill`, `cast`, `round_start`). Effects on
the match are requests that Rust applies at the start of the next tick,
within quotas: Growing Guns' `g:damage`, `g:give` and `g:explode` (32
explosions a tick) return false over the quota. Luau can describe what
should happen; Rust decides how and protects the frame.

### Luau sets parameters, Rust runs the algorithm

Pine lab (`games/pine-lab/main.luau`) is a panel of growth rules. Luau
holds the parameter list and the UI; `plants.grow` hands a `Species` and
`Look` to Rust, which grows and cooks the tree on background threads. The
expensive part never runs in Luau, and adding a parameter is one line of
Luau.

### Keep a small game's state in Luau

Neon Siege keeps its enemies and shots in Luau tables and moves their
render entities with `world.place`. For a few hundred things that is
simple and fast enough, and every rule hot-reloads. When a game outgrows
that, move the loop (next section), not the rules.

## Moving code from Luau to Rust

Start in Luau. Move when a profile says so, not before.

1. **Measure first.** A slow Luau hook usually shows as a long tick in
   the [stats overlay](stats-overlay.md) or as a step-budget error.
   Neither the overlay nor the log names the Luau hook yet; time it with
   `game.time` around the call or bisect by commenting out.
2. **Move the loop, keep the decision.** Write the per-entity work as a
   Rust system that takes its parameters from Luau: a `steer(ids, params)`
   that moves every enemy, a `separate(tag, radius)` for crowds, a batch
   query that returns all hits at once. Luau still says what the enemies
   want; Rust does it for all of them.
3. **Keep the Luau API the same shape** where possible, so mods and other
   games keep working, and keep the numbers in Luau.
4. **Put the data that tuning produced in a data file, not in Rust
   source.** Values copied from Luau into a Rust constructor (Pine lab's
   advice to copy tuning into `Species::jeffrey_pine()`) lose hot reload
   for everyone after. Prefer the Luau or JSON file as the source and
   `include_str!` it as the built-in default.
5. **Do not keep two copies of a rule.** Spellstorm's sim has the built-in
   grammar in Rust and its port in `mods/base/spells.luau`, kept equal by
   a test. That is double work for every rule change. When a rule lives in
   Luau, ship the `.luau` file as the built-in default (Spellstorm already
   embeds it as `BUILTIN`) and let tests run the Luau.

Going the other way, out of Rust into Luau, is worth it for any constant or
rule you find yourself rebuilding to tune.

## What a Rust change costs

On the same 4-core VM, with the workspace's dev profile (`opt-level = 1`
for workspace crates, dependencies at `opt-level = 3`, incremental):

| Change | Time |
| --- | --- |
| First build of the Spellstorm sim crate (`cargo build -p spellstorm --lib`) | 70 s |
| First build of the Spellstorm game binary (`cargo build -p spellstorm-app`), sim dependencies already built | 2 min 45 s |
| Add a function to the sim crate, rebuild the game binary | 5.5 s |
| Add a function to the game's app crate, rebuild the game binary | 5.4 s |
| Add a function to `render` or `kern`, rebuild the game binary | 16.5 s |
| A comment-only edit anywhere | 2 to 5 s |

Then the game restarts and has to be played back to where it was. A Luau
edit in the same game is live in a quarter of a second, mid-match. The gap
is not the five seconds of compiling but the restart and replay around it,
dozens of times an hour.

So:

- **Keep game rules in a sim crate with no renderer dependency**, as every
  Rust game here does. Its tests and headless scripts rebuild in about a
  second.
- **Avoid editing `render`** while iterating on a game; everything above
  it rebuilds and relinks.
- Restarting the game loses the session; scripted playtests
  (`script s.json`) and F12 captures (`--from`) bring a scene back.

## Where the split is weak

Known gaps, so nobody mistakes them for rules:

- **No Luau timing in the stats overlay.** A slow hook is hard to find.
- **The step budget's interrupt costs ~30 ns per loop iteration**
  (measured above). A cheaper counter would make tight Luau loops several
  times faster.
- **The `warp` API builds its answer tables under the memory cap**
  (`world.get`, `world.raycast`, `world.overlap`), so Luau games pay the
  slow path described above on every query.
- **Few batch calls.** The Luau world API is per entity (`world.move`,
  `world.get`); there is no call that moves or queries many entities at
  once.
- **Not exposed to Luau yet:** physics bodies, terrain, skinned animation
  and local lights (see [not yet](warp-cli.md#not-yet)). A game that needs
  them is a Rust game today.
- **Luau numbers are f64, engine vectors are f32**, and `math.sin` and
  friends come from the platform's C library. The same build replays bit
  for bit; replays across platforms (lockstep online play) have not been
  checked for Luau. Sideways uses `gdcompat` in Rust for this reason.
