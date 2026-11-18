# Warp Humans posture in Luau games

18 November 2026, on `5148dc1`. Cloud container, lavapipe (software Vulkan),
1280 × 720, `games/human-lab/scripts/portraits.json` and `moving.json`.

The first close captures of Luau humans (Dead Miles' survivors) showed
shoulders at ear height, the head sunk and tilted down, a bent back, elbows
out and feet wide. Three causes, each measured:

1. **The body was stretched onto the animation rig's joints.** The rig's
   shoulders are 0.57 m apart at 1.53 m; the hm08 body's, at 1.80 m, 0.36 m
   apart at 1.48 m. `warp_human_with_skin` moved every arm vertex out and up
   to the rig's shoulder. Luau humans now use `warp_human_person`, which
   keeps the spine, head and arm joints where the body has them (as Growing
   Guns' players already did) and gives each person their own skeleton for
   the captures.
2. **The retarget turned trunk segments.** Aligning each rig segment with
   the capture's direction is right for arms and legs, but along the spine
   it only compares where two skeletons happen to put their joints: 10° at
   the upper back, 6° at the neck, and the collarbone's turn lifted the
   shoulder joint about 8 cm. The trunk, neck and collarbones now keep the
   body's own rest.
3. **`Idle_Loop` is a fighter's stance.** At 0.5 s the thighs are turned
   29°, the pelvis 14° and the chest 15° forward. People standing still now
   blend to the upright rest with breathing, weight drift, glances and a
   slight elbow bend; the zombie style keeps the capture.

`Walk_Loop` also leans the chest 17° and the head 15° forward; left as it is.

Checks: `cargo test -p warp humans` (5 pass, one new for the stance),
`warp check` for Dead Miles and Human lab. No real GPU run.
