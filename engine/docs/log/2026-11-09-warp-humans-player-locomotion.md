# Distance-matched locomotion on the player rig

On 9 November 2026 we tested Warp Human players on Mancave (Windows, RTX 5070 Ti) against the wardrobe build at `43eb8678`. The old pistol run advanced every tick with a playback multiplier clamped to 0.75–1.5. The jump sampled a single non-looping clip by elapsed time and switched straight to a ground pose on contact.

The [CMU Graphics Lab motion database](https://mocap.cs.cmu.edu/) and [Quaternius Universal Animation Library](https://quaternius.itch.io/universal-animation-library) both cover human locomotion. Growing Guns uses the latter's existing CC0 engine tracks. Walk, jog and sprint bind to the player's pelvis, thighs, shins, feet and toes. The weapon pose follows the animated pelvis through the existing hand and gun attachments.

The left toe passing under the pelvis on the support stroke supplies a common phase reference. At that point the change in model-space toe position per cycle gives the metre scale for each loop. Measured on the donor skeleton, the values were 1.18 m for walk, 6.12 m for jog and 6.51 m for sprint. The units are effective travel per full left-to-left cycle. A regression check advances the root by 3 cm around midstance and checks that the toe's remaining travel along the motion direction is below 6 mm. It exercises forward, backward and lateral movement at 1.2, 5 and 14 m/s.

The upper portion of the jump-start clip supplies knee lift. Actual vertical velocity places the ascent, apex and extension. An initial capture showed why the stationary landing needed a separate blend at speed: a runner kept covering ground while the landing pose stayed planted. The moving landing now puts the next supporting foot into its distance-driven cycle at contact. Double jumps can return from extension to ascent, and stepping off an edge uses the same aerial states.

For visual verification we drove a second local player with real movement and jump input in a full-graphics seed-52 match, arena 13. A side camera followed the player over the same paving for walking, jogging, acceleration, takeoff and landing.
The release capture completed all 242 simulation ticks with 101 frames at 30 fps. The player reached 1.43 m above standing height at the apex and returned to the ground on frame 78 while still moving forward.
