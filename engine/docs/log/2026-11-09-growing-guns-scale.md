# Growing Guns world scale and eye-height playtest

On 9 November 2026 the previous match build (028f1441, seed 52, Windows, RTX 5070 Ti)
used roughly human-sized people in a 90 m arena. The standing capsule is 1.80 m high
and the standard first-person camera is 1.60 m above its feet. Weighted vertex bounds
in the first pistol-idle frame measured 1.69 m for ch10, 1.88 m for the knight and
1.74 m for the paladin. The original Warp port already used a 1.80 m capsule.
Commit 43dc91bf raised the eye offset from the capsule centre from 0.62 to 0.70 m.

The player in the saved screenshot was descending at 23.58 m/s and had no ground contact.
This frame cannot measure standing eye height. The old placement sampled a curved radial
row across rectangular tread segments. Near some corners this put the person one row
above or below the physical tread. One old riser measured 1.40 m.

New matches build the architecture at 80% of its authored dimensions. Seed 52 now has
a 72 m arena, 9.60 m perimeter walls and 1.12 m risers. Rendering, collision, island
connections, light and spawn locations all use the same layout. Player spawn clearance,
spectator dimensions and card-based player variation are independent of this scale.
The match places the spectators on the flat treads, resolving overlaps against the
walkable geometry. Saved arenas record the geometry scale; the previous four wire-layout
variants reconstruct the original stadium dimensions.

For the standing comparison, run `play --warp-humans --start-in-stands --skip-menu
--seed 52` with the desktop Growing Guns executable. The first local player starts
in a gap facing a neighbour horizontally. The same start is available with `shot`.
