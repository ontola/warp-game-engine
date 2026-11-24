# Sedan door reflections at different distances

In the public `c0682158` playtest, highlights took sharp diagonal turns across both doors of the ochre Toyama. The pattern was also visible on the sunlit portion of a car straddling a building shadow.

On the Mancave RTX 5070 Ti, an Edge WebGPU camera looked at the broadside of the sedan from 10.1 m. With the original paint triangles the reflected line ran continuously along the panels. Switching to the stock geometry-only cluster cut produced the diagonal highlights. Both renders used the shipped material. The paint primitive contains 6,819 vertices. Its positions, normals, UVs and indices match the Sideways source. The four paint colours share the same geometry buffer.

The city opts its painted panels into the static glTF loader's shading-preserving simplification. Vertex normals participate when choosing which triangles to collapse. The city uses the same paint parameters and textures as before.

The shipped policy at `887b656d` and `5cb5a409` uses normal weights of 0.15 and a smaller screen-space bound on triangle orientation changes for these opaque surfaces. At 10 m in the 1,088 × 612 broadside view, paint used 5,568 triangles. Its 95th percentile normal error against the full mesh across 2,754 door samples rounded to 0.0°. At 40 m the paint used 2,162 triangles, and at 80 m it used 270. With the original positional simplifier, 67.2% of close door samples differed by more than 15°.

In the seeded chase camera, 12 of the 43 cars intersected the view frustum. The paint cut went from 3,087 to 6,778 triangles with the selected policy. Full source geometry would have drawn 103,780. These counts precede building occlusion. An earlier option kept 34,392 triangles in the same chase view.

A second Edge comparison locked resolution at 1,088 × 612 and held the camera, city and lighting fixed. Five views included a car crossing a building shadow. Against the high-detail reference, mean RGB channel RMS differences on the 10 m doors fell from 28.55 to 1.78 on a 0–255 scale. Across the sun–shade transition the error fell from 7.33 to 0.84. Texture and geometry downloads are identical. Four small material extras select the new simplification path.
