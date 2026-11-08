# Screen-space refraction, 8 November 2026

Base commit 310872a. Cloud container, lavapipe (software Vulkan, not a real
GPU). Opt-in (`KERN_REFRACTION=1` or `Scene::graphics.refraction`) until
checked on Mancave.

## What was built

Blended materials can carry a `Refraction` (index of refraction, thickness).
With refraction on and refracting glass in a view, the transparency pass
becomes:

1. copy the opaque depth, draw the refracting triangles depth-only over it:
   the nearest refracting surface per pixel;
2. draw the sorted glass list, keeping only fragments behind that surface
   (or everywhere without one);
3. copy the HDR (opaque plus that glass);
4. draw the nearest refracting surface: reflection plus Fresnel-weighted,
   hue-tinted transmission of the copy at the bent position, replacing the
   destination; then the glass nearer than it.

The phase sits in the top two bits of each sorted order entry, so one
pipeline and one order buffer serve all draws. The copies, opaque depth
(read-only attachment) and front depth live in a fourth bind group with no
storage buffers; without refraction the pipelines are built without it.

## Rejected: one pass with a delta against the opaque copy

The first version kept one pass and wrote `T * (bent - straight)` on top of
premultiplied blending, so glass behind kept its light and only the opaque
background moved. Where darker glass lay behind (a windshield seen through
a side window in Sideways) the destination was darker than the opaque copy,
the subtraction went negative and the red car behind turned cyan. The
front-surface split above has no negative terms.

## Results (lavapipe)

- `refraction_probe`: glass ball (solid), 6 cm slab, 4 mm pane in front of a
  striped wall and checkered floor. 8380 of 256000 pixels change by more
  than 8/255; split screen gives identical halves.
- Sideways `shot --cam yaw=70,dist=3.6,height=1.3,car=0,fov=45` at 960x540:
  window glass changes slightly (4 mm panes barely bend light; the visible
  part is the Fresnel reflection and the shift of the other car seen through
  two windows). Transparency pass 24.7 ms off, 33.8 ms on, on lavapipe:
  meaningless as a GPU cost; measure on Mancave.
