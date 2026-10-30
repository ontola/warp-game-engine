# Balance

How Spellstorm keeps attacks from running away with the game. Ported from
the Godot game's docs/balance.md: the rules are the same, the names now
point at the Warp port. The numbers live in `sim/src/balance.rs` (the
`Balance.X` and `Grammar.X` names below are its constants), the spell rules
in `mods/base/spells.luau`, the tomes in `mods/base/tomes.luau`.
`sim/tests/spells.rs` (`balance_band`, `every_reachable_spell`) checks every
reachable spell against them, and CI fails when one leaves the band.

## The exchange rate

**1 mana buys 9 damage** (`Balance.DMG_PER_MANA`).

Every spell is held to that rate. Bolts and the staff are not, because they
are the free attacks; they have their own budget (see below).

## What a spell costs

The price is what the player really pays:

- **loading:** 5 mana per element put in the queue (`MANA_PER_LOAD`);
- **casting:** 13 mana per element still in the queue (`MANA_PER_ELEMENT`).

The queue holds three elements (`SpellQueue.SIZE`). It used to hold five; each
element now weighs 5/3 of what it did (`Grammar.STEP`), in cost, in ammo and in
how big a spell it makes, so a full queue is what a full queue was. One
element is 18 mana and must be worth about 160 damage. Three elements are 54
mana and must be worth about 490 damage, plus the combo bonus. A full queue
cast ahead is a cataclysm, planted a sentry.

## The combo bonus

A longer or more mixed queue takes more work to build, so it may hit a
little harder:

- **+10% for each element past the first** (`BONUS_PER_SLOT`);
- **+6% for each different element past the first** (`BONUS_PER_KIND`).

| Queue | Target |
|---|---|
| Fire | 1.00 |
| Fire Fire | 1.10 |
| Earth Void | 1.16 |
| three slots, three kinds | 1.32 |

## What a spell is worth

`Balance.worth()` adds up everything a spell does over its whole life, to
everything it is likely to hit:

- **Direct damage:** `El.EFFECT` dmg × the count of each element × how much
  of the spell lands. A spray lands every second of its duration, a
  thunderbolt or a nova once.
- **Burn:** 7 damage a second, at most 6 seconds past the last hit. More
  hits add time up to that, never more damage a second. Water or frost puts
  it out.
- **Freeze:** worth 12 damage, since it holds a monster still.
- **Utility, in damage per unit:** push 1, wet 1, slow 3, shock 2. Wet and
  shock are combo enablers (shock does double on wet), so they count, but
  not as much as damage.
- **Poison:** a dose of damage that seeps out: each unit of scale adds 25,
  and the dose drains a fifth of itself a second, so it fades instead of
  stopping (the last bit at 2 a second). Doses stack up to 150 on one
  monster. Nothing washes it off. No spell heals: health comes from blood
  that monsters spill and jars hold.

### Damage that comes later counts less

A monster burning for six seconds may die of something else first, walk
out of reach or be doused. So damage over time is credited by when it lands:
damage t seconds after the hit counts `exp(-t / PATIENCE)`, PATIENCE 4 s.
While a lasting spell keeps it topped up it counts in full, like any hit;
only what runs on after counts less.

- A full 6 s burn deals 42 and is credited 31.
- A poison dose comes out on average 5 s late and is credited 4/9 of itself:
  a dose of 100 is credited 44.

The budget pays for credited damage, so the longer an effect lasts, the more
it really deals per mana. Poison lasts longest and is the strongest element:
`tests/balance_test.gd` prints real damage per mana for spells led by each
element and fails when poison is not on top (now poison 1.30, fire 1.07,
the rest 0.8 to 1.0).

### How many monsters a shape hits

The expected number of monsters follows from the area a spell covers: one,
plus **0.04 monsters per square metre** of it (`DENSITY`), at most 6. A
bigger nova or a wider cone counts on more.

| Shape | Area | Expected |
|---|---|---|
| nova | the circle | 2.2 to 5.0 |
| trap | the blast circle | 1.8 to 2.6 |
| spray | the cone (angle × range² / 2) | 1.2 to 1.7 |
| projectile | 1, plus 0.6 of the splash circle | 1.0 to 2.1 |
| strike (thunderbolt) | 1, plus 0.6 of the splash circle | 1.1 to 1.4 |
| beam | stops at the first | 1 |
| sentry | picks one, in range 60% of the time | 1 |

### A crowd counts for half

Nothing is scaled while you play: every monster in the flames takes the
same damage, however many there are. But in the sum that sets a spell's
strength, **the first monster counts fully and every one after it for half**
(`CROWD`). A flamethrower expected to hit 2 counts as 1.5. So on one monster
alone an area spell is never stronger than a single-target spell of the same
mana, and on a crowd it is worth more.

### Time you cannot shoot is a cost

Bolts are free, so a spell has to beat what you would have done instead.
Every RT spell winds up first (`WINDUP`), and while you hold a flamethrower
or beam you cannot shoot at all. A spell's gain is its worth
**less 12 damage a second (`BOLT_DPS`) for every second the staff is busy**.
A flamethrower does a lot a second, but you also stop shooting for its whole
length, and the sum knows it.

### Hard spells earn a little more

`EASE` multiplies a shape's target: an aimed boulder that is slow to land
gets 15% more, a sentry that aims itself 15% less, a spray 5% less.

### Short reach earns a little more

Using a short-range attack means walking into danger. Sprays, beams and
thunderbolts, and every element's bolts, are even at 14 m of reach
(`RANGE_EVEN`) and get 20% (`RANGE_WEIGHT`) more for every 10 m less, or
that much less for every 10 m more (at most 10% less). Poison globs that go
10 m hit 8% harder for it, frost needles that go 22 m 10% softer. Slow bolts
already count as harder to land (`skill`).

## How it stays balanced: fitting

`Grammar` builds every spell's size, range and duration as before. Then
`Balance.fit()` scales the spell's `rate`, which is the strength of each hit,
so its worth / (price × 5) lands exactly on its target.

Shapes and elements keep their character: a flamethrower still burns, a
boulder still shoves and a frost nova still freezes. They just cannot be
worth more than the mana you put in. The factor is worked out from the spell
without relics, so a relic still makes you stronger.

Tuning:

- Want all spells stronger or weaker? Change `DMG_PER_MANA`.
- Want one shape to do more? Change its `TARGETS` or `UPTIME`. That says the
  shape is worth less than it looked, so fitting gives it more.
- Want combos to pay more? Raise `BONUS_PER_SLOT` / `BONUS_PER_KIND`.

`cargo test -p spellstorm --test spells balance_band -- --nocapture` prints every spell with its worth split
into hit, burn, poison, freeze, utility.

### Not fitted

- **Cataclysms** (any full queue cast ahead: Meteor, Tempest, Black Hole and the rest) are
  hand-made showpieces in `OverloadFx` at a fixed 40 mana plus loading.
  They may be stronger than the budget, because they empty your queue.

## Enchantments: a store you shoot up

An enchantment (RS) is priced like a spell: loading plus 8 mana an element.
It turns that into a **store of damage** of price × 9 × its target
(`enchant_store`). Every enchanted bolt and every enchanted swing spends what
it is worth from the store (`enchant_shot`, `enchant_swing`), counted the
same way as everything else. When the store is spent the sword is steel
again; it also ends after 20 seconds. So it is a short, strong buff: an
enchanted bolt hits 2.5 times as hard as a loaded one (`ENCHANT_BOLT`), and
the harder you shoot, the faster it is gone. As it runs low, the element
draws back to the sword's edges.

`tests/balance_test.gd` prints how long a few enchantments last while you
only shoot, and how many swings they are, and fails when one lasts under 2
seconds or past the time limit.

**Sentries** (planting a full queue of three) are fitted like the rest, but
cost a fixed 50 mana plus loading (`SENTRY_COST`), so they are the big plant.

## Bolts: the free attack

A bolt costs nothing to fire. Every element's bolt stream is worth about
**12 damage a second** (`BOLT_DPS`), counting status and utility the same
way as spells, a little more when its bolts are harder to land (`skill`:
slow bolts, few a second, thrown in an arc, or short of reach). Each element flies its own
way (`FLIGHT`):

| Element | Shots/s | Speed | Range | Arc | Feel |
|---|---|---|---|---|---|
| Fire | 6 | 22 | 16 | | the steady stream |
| Frost | 4 | 36 | 22 | | fast, far needles |
| Water | 9 | 16 | 9 | yes | a short hose |
| Lightning | 5 | instant | 11 | | a spark from the staff, no projectile |
| Earth | 1.5 | 15 | 13 | yes | a heavy thrown rock |
| Air | 7 | 30 | 18 | | quick gusts |
| Poison | 3 | 11 | 10 | yes | slow lobbed globs |
| Void | 2 | 9 | 15 | | slow, heavy orbs |

A fusion flies like the average of its two parts. The strength of one bolt
follows from that (`bolt_scale`), so a rock hits about five times as hard as
a fire spark. Loading an element (5 mana) gives 2.25 seconds of its bolts
(`AMMO_TIME`): 14 fire bolts, 3 rocks. In a mixed queue the front element sets
how the bolt flies; every element in it keeps its own worth a second.

A plain bolt (empty queue) does 0.12 of an earth hit: weak on purpose, so
loading is worth it.

## Where mana comes from

**Souls.** Every monster that dies drops blue mana shards worth
**0.15 mana per point of its health** (`SOUL_MANA_PER_HP`). A 40 HP ghoul
gives 6 mana, a 240 HP brute 36, and a boss caps at 10 shards. Walk over
them to take them; close by, they drift to you. A wizard with full mana
leaves them for the others. They fade after 14 seconds.

Besides souls:

- regen of 3 mana a second, so an empty wizard is never stuck for long
  (`MANA_REGEN`);
- 3 mana for a staff hit, and 3 for a deflected bullet.

Bolts no longer give mana. Going in for the souls is the risk that pays for
your spells.

### The loop in numbers

A pack of about 600 HP of monsters drops about 90 mana. Spending it all on
spells at 5 damage per mana kills about 450 HP. Bolts and the staff have to
do the rest, so the souls of one fight pay for most, but not all, of the
spells in the next.

## The staff swing

RB swings the staff in an arc of ±75° and 2.6 m (`SWING_ARC`,
`SWING_REACH`). It knocks enemy bullets back at twice their power. You
cannot shoot or cast during the swing (0.34 s). After it, the staff needs
0.7 s to recover (`SWING_RECOVER`): the gem goes dark and flashes when it is
ready again.

## Tomes

A shrine offers one tome per wizard plus one. A tome changes numbers in a
way you can say in one sentence; it never changes what a spell is. The list
is in `mods/base/tomes.luau` (hot-reloads) and, as the built-in copy that
`sim/tests/luau.rs` keeps equal to it, in `sim/src/relics.rs`.

### Only tomes you can use

A shrine offers only tomes that someone standing there can use
(`relics::usable`, `Mod::need`):

- an element tome needs one of its elements learned (a fused element both
  its parts), so a new wizard with fire and lightning sees no Poison tome;
- Echo Stone needs the nova, Mason's Chisel the plant, Runed Staff the
  enchant;
- everything else (health, mana, speed, damage taken) always helps.

Bots know everything, so they only count when no person is playing. When
fewer usable tomes are left than altars, the rest are filled from the others.
`sim/tests/spells.rs` `shrines_offer_only_tomes_you_can_use` checks it.

### The budget

Spells are fitted without tomes, so a tome is pure profit. To keep a pick
between tomes a real choice, every tome is worth about **+25% of what it
touches, for the wizard who plays to it**:

| Reach of the tome | Size | Examples |
|---|---|---|
| Everything you do | +15% | Glass Staff (with -30 health), Blood Pact +25% (you take +25%) |
| A family of 3 to 4 elements | +35% | Ember and Rime Codex, Storm Sigil |
| One element and its fusion | +50% | Pyre Psalter, Frozen Heart, Thunder Rod |
| One element | +60% | Tomb Dust (poison) |
| One habit (a form, the staff, bolts, souls) | x1.4 to x2 | Keen Gem, Runed Staff, Quicksilver Grip, Soul Jar |
| Staying alive | about one extra hit taken per fight | Thick Hide +30, Iron Skin -20%, Wayfarer's Charm |

Mana tomes are measured in damage through the exchange rate: +40 max mana
(Deep Well) is two more one-element spells per fight, about 320 damage; Moon
Font doubles the 3 mana/s regen, about +27 damage a second while you keep
casting; Miser's Ledger (-20% cost) is +25% spells for the same mana.

A downside may buy a bigger number (Glass Staff, Blood Pact), never a
smaller one. No tome heals: health still only comes from blood
(Red Chalice makes that blood heal more).

### Ideas not built yet

Kept here so the list can grow without losing the thread:

- **Cataclysm** tomes: a full-queue cast ahead costs 25% less, or its area is
  larger (needs `Mods` hooks in the overload code).
- **Sentry** tomes: sentries last longer, shoot faster, or carry 2 elements
  of the queue's power (needs hooks in the sentry code).
- **Chain** tome: lightning jumps to one more monster.
- **Nova** tome: bursts around you reach 30% further.
- **Second wind**: the first time you would fall in a level, you keep 1 hp.
- **Faster raising**: you raise a fallen friend twice as fast.
- **Pack tomes** that help everyone at the shrine a little (+10% each),
  for co-op choices.
