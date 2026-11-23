# Port Meridian objective markers and street combat

Verified on Windows with an RTX 5070 Ti, city seed 1948, against the v1.1 playtest based on `b997ee65`.

At 1280 × 720, an on-foot camera at the Ferry Terminal saw the nine-metre dispatch circle. A driver in the pearl Comet saw all four posts around the first Exchange Street Sprint checkpoint. A 1440 × 900 split-screen replay observed Mara from opposite sides as she walked toward the curb. CPU checks confirmed both cameras share one world anchor, an authored 4.5 m gate uses a 9 m diameter, and crossing it removes the marker.

The shooting replay used normal shoulder aim and two trigger presses against a patrolling rival. The first hit produced a directional spray and a ground mark. The second removed the rival's red marker, activated its ragdoll and added bouncing fragments. A two-player test split the hits between seats and checked cover, per-seat cooldown, shot ownership, effects expiry and respawn. The four-room streaming test peaked at 1,172 world entities with four players, under the configured 1,280.

The two rival slots use a separate Warp Humans pool. Combat reuses the existing story cast for walking and ragdolls. Three prewarmed game models adapt the Growing Guns droplet, cut fragment and splat artwork. The effects module shares limits of 16 flying drops, seven fragments and eight ground marks across all seats and stories.

[Player controls and story format](../../../games/warp-theft-auto/README.md)
