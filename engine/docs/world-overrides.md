# World overrides

Step 3 of the constrained procedural worlds roadmap: artist edits on top of
rule placement (`render::placement`), keyed on the stable object ids.

- `render::overrides::Overrides`: one `Edit` per id, which is one of:
  - `Remove`: the object is gone.
  - `Replace(Pinned)`: it stands as pinned (moved, turned, rescaled, another
    variant or rule), whether or not its rule still makes it.
  - `Add(Pinned)`: an object no rule makes. Its id has rule field `0xffff`
    (`overrides::ADDED`, kind 255 and rule 255, which no rule may use),
    numbered from 1.
- `Pinned` keeps the height above the ground (`lift`), so an edited object
  still stands on the land when the terrain changes. `Pinned::of(instance,
  world)` pins a placed object as it stands.
- `Overrides::apply(placed, rules, world, centre, radius)` runs after
  `placement::place`. Removed and replaced objects leave; pinned objects of
  those rules inside the disc join. The other objects keep their order.
- Saved as plain text, one edit per line, sorted by id, so diffs stay
  readable (`to_text`, `parse`):

```text
# warp overrides 1
remove 0001000000500007
replace 0001000000300004 rule=1 x=12.5 z=-40 lift=0 yaw=1.2 scale=1,1.3,1 variant=2
add ffff000000000001 rule=2 x=30 z=8 lift=-0.2 yaw=0 scale=2,2,2 variant=0
```

## Try it

`terrain_fly --write-overrides FILE` writes and uses an example: the 30 trees
nearest the start are removed, the next one moves 6 m, and five trees are
added in a row 20 m away. `terrain_fly --overrides FILE` reads a file. Both
apply to trees and shrubs (`--at 0,0,45,0,-1.2` looks down on the clearing).

## Validation

- `overrides::tests::edits_survive_a_tweaked_rule`: the edits land on the
  original placement and on a sparser tree rule (a replaced tree stands where
  its rule no longer grows one), a moved object keeps its height above the
  ground, added objects outside the disc stay out, pinned objects only join
  their own rules, and untouched objects keep their order.
- `overrides::tests::text_round_trips_and_rejects_mistakes`.

## Not yet

- Placement runs on the CPU, so there is no GPU id table yet. It comes with
  GPU placement (roadmap step 4).
- The far forest drawn into the terrain (`forest_field`, past the handover
  band) does not know the edits: a tree removed out there still shows as
  painted canopy.
- Grass and the town's buildings are not placement objects, so they take no
  overrides.
- No editor: files are written by hand, by `--write-overrides`, or by a
  tool. Live authoring is roadmap step 10.
