# Stadium atmosphere on Warp Humans

9 November 2026. Growing Guns, Windows, NVIDIA GeForce RTX 5070 Ti.
The baseline was the blood-spray build at the source state in `5e102853`.

The match drew spectators into the HDR image after the opaque stadium had
already received distance fog. Both close meshes and distant impostors
therefore retained their full surface contrast against fogged steps.
The crowd fragment now evaluates the stadium's shared atmosphere function
on the ray from the current camera to the animated surface. It reads the
same view, sky, shadow and lighting resources as the masonry. This adds no
draw or fullscreen pass. The standalone appearance viewer keeps its own
lighting and binds no scene atmosphere resources.

Thirteen targeted renderer tests passed, including both human fragment
entry points, zero and full fog, finite-height scattering and split views.
