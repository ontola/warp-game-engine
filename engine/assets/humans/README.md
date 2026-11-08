# Warp Humans reference head

head.bin derives from the body and eye helpers of MakeHuman hm08 base.obj,
revision a8bc2d54ff0ac92e78ff71431b1023eda42bf482. The source header dedicates
this asset to CC0 (2020, Data Collection AB, Joel Palmius and Jonas Hauquier).
https://github.com/makehumancommunity/makehuman/blob/a8bc2d54ff0ac92e78ff71431b1023eda42bf482/makehuman/data/3dobjs/base.obj

Coordinates are original OBJ positions. Retained triangles have every vertex
at y >= 5.55 and |x| <= 1.31. Quads split along their first diagonal.
Two little-endian u32 counts precede float32 xyz positions and four-u16 faces
(a, b, c, eye flag). Vertices use ascending original index. No source texture.
