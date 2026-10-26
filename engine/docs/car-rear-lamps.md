# Rear-lens emission

The CLI tags original rear-lens triangles during GLB import. It does not add
quads, move vertices, duplicate triangles, alter collision or illuminate the
ground. Lamp slots belong to each car, even when four cars share one model.
Authored normals, UVs, textures and transparency remain intact.

Measured imported body-space geometry (before the normal ride translation):

| Model | Source material | Triangles | Y range (m) | Z range (m) |
| --- | --- | ---: | --- | --- |
| Asti | UCB_Lights_and_Glass.003 | 252 | 0.266–0.540 | 1.788–1.856 |
| MV6 | UCB_Lights_and_Glass.002 | 76 | 0.393–0.624 | 1.710–1.875 |
| Toyama red | Body_Glass_Red.001 | 276 | 0.352–0.610 | 1.854–2.104 |
| Toyama reverse | Body_Glass.001 | 144 | 0.352–0.473 | 2.074–2.111 |

The two atlas cars retain separate unlit glass covers, avoiding duplicate
emission. Their original atlas colours distinguish red, white, amber and dark
surrounds per pixel. Toyama uses the authored separate red and clear lens
materials; its rear window is outside the selected geometry. Unknown models
remain unchanged rather than guessing lamp positions.

`car_lamps.rs` reads actual simulation state. Tail lamps follow the same
twilight/night policy as headlights. Foot braking activates brake lamps in
daylight too. Automatic reverse uses RT/throttle as the effective brake, just
as `Car::drive` does; LT/brake driving the car backwards does not incorrectly
turn on brake lamps. Reverse lamps depend on the engaged gear, including
standing still in reverse. The finish hold applies the foot brake.

The existing per-car material buffer carries the state: kind 7, `second.xy`
for red/white peak radiance and `second.w` for lens role. Material data is
uploaded only when lamp state changes or a live material edit requires it.
Paint-data reloads restore lamp state before presenting the next frame. F12
and shot metadata include the applied lamp states and their model/slot IDs.

Initial tuning is 600 cd/m² tail, 4000 brake and 1800 reverse peak radiance,
modulated by the original lens texture. These are art values, not measured
vehicle photometry. Emission is added before distance fog and exposure in
both opaque and transparent passes. It is not multiplied by sun visibility,
so mountain shadows cannot turn an emitting brake lamp off. No bloom,
rearward ground lighting or atmospheric beam has been added.

The shader adds no texture reads, bindings, shadow maps or extra instances.
Material splitting can change cluster packing; its actual GPU cost and the
day/night appearance still need matched render verification. Source checks
cover effective pedal semantics, four independent cars, unchanged-state
uploads, red/clear isolation, exact GLB triangle counts and window/front
exclusion. At initial delivery these checks are added but await Cargo in the
coordinated build; no GPU acceptance is claimed.
