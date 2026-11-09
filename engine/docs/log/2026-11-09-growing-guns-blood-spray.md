# Blood spray on the stadium steps

9 November 2026. Windows, GeForce RTX 5070 Ti. Compared the crowd reactions in
`d5073716` with the accompanying Growing Guns changes in the packed stadium,
seed 52, full graphics at noon. The first-person replay included gunfire and
a grenade. It captured 960 × 540 images every fourth tick over 228 ticks.

The first mesh-only shapes produced distinct silhouettes but had conspicuous
straight edges on the treads. The final material uses one shared atlas with
16 seeded silhouettes. Small detached drops and gaps let the concrete show
through the spray. Marks follow the struck face, including vertical risers.
The review compared matching frames on the same steps.

The stadium reuses its 768 mark slots. Secondary contacts reuse the arena's
384 mark slots. Selected flying debris can stamp a surface once on contact.
The particle regression checks that a consumed contact frees its slot, and
the physics regression checks the direction and position of a ground contact.
The final atlas build was also launched in a regular one-local-player match.
Its incremental GPU frame time was not measured.
