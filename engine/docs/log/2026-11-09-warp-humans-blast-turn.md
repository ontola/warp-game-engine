# Turning away from a blast

9 November 2026, Windows, Growing Guns crowd. Baseline `5ffac5f2`.

Surviving spectators turn away from the epicentre in their own facing
frame. A directly frontal blast lets each person lead with either shoulder.
The reach takes roughly half a second for a large first turn, holds for
0.48 seconds, then eases back. A second blast samples the current angle
before selecting the new direction. The rigid crowd turns at its root.
The rigged crowd shares the angle between hips, shoulders and neck while
its existing footwork continues along the steps.

The persistent reaction record grows from 32 to 40 bytes per seat.
The view packs the turn beside the blue light channel in the existing
48-byte visible record. The selection and draw passes carry the animation
at each distance.

A four-seat check ran the production reaction shader on the Microsoft
Basic Render Driver. At 0.32 seconds the two nearby survivors had opposite
turns of 1.206 radians. A passing bullet and a second explosion at that same
instant both preserved their angles. The second blast reversed one person's
turn. Both faced forward again afterwards. The distant spectator and the
spectator at the lethal centre recorded zero turn. The review clip uses
32 characters on staged steps, rendered in software at 960 × 540.
