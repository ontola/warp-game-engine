# Planted steps and crouches on the terraces

9 November 2026, Windows. Compared with the gesture build at `32e09561`.
The new pose uses the existing continuous escape displacement. Signed row
travel determines alternating footfalls. The stance foot cancels the root's
sideways motion and the swing foot lifts, advances and settles. Walking out
and back uses the same trajectory. A partial-speed weight eases to neutral
at rest. A frightened spectator lowers the pelvis by up to 22% of height
and bends the legs. Shoes, head and limb segments retain their dimensions.

Hip pitch and roll, knee flexion and ankle counter-rotation are applied to
the weighted close mesh and to the lower-detail parts. The far leg uses
one splat between hip and ankle. The compacted pose and lighting record
occupies 48 bytes per slot across the existing five bins. Joint selection
and drawing use the existing passes.

A CPU target check evaluated the shader's scalar two-link solve including
half-precision angle packing. It sampled 4,001 signed stride phases, three
stride lengths, both feet and three crouch depths at full walking weight.
At 1.95 m body scale, the largest ankle-target error was 0.48 mm. The
largest stance-axis error was 0.41 mm. This checks the joint maths only.
No game window, GPU rendering or frame timing was used for this check.

The packaged recordings and their original mixing levels were checked
against the source bank and the mixer in `72f356bf`.
