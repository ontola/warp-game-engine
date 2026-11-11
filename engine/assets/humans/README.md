# Warp Humans close character sources

The reference body, fitting data and textures derive from MakeHuman hm08 and
the MakeHuman system asset pack, released under CC0. The source copyrights
identify Data Collection AB, Joel Palmius and Jonas Hauquier.

- [hm08 mesh, deformation targets and skinning weights](https://github.com/makehumancommunity/makehuman/tree/a8bc2d54ff0ac92e78ff71431b1023eda42bf482/makehuman/data)
- [MakeHuman system pack, asset list and licences](https://static.makehumancommunity.org/assets/assetpacks/makehuman_system_assets.html)

The cooker uses base.obj, default.mhskel, default_weights.mhw and the targets listed in
`engine/tools/cook-humans.py`. Place the weight and target files together.
Extract male_casualsuit01 (pullover), male_casualsuit06 (short sleeves),
male_casualsuit05 (jacket), shoes02, eyebrow001, short01, bob02, afro01 and the
three listed skin materials into the system pack's original directory layout.
The cooker fits clothing and groom vertices from the mhclo barycentric bindings
and transfers their skin weights and facial displacements from the same body.

```sh
python engine/tools/cook-humans.py base.obj source-files/ system-pack/ engine/assets/humans/
```

The neutral eyelids use the six paired MakeHuman targets in
`makehuman/data/targets/eyes/{l,r}-eye-height{1,2,3}-decr.target`. Keep
them beside the face targets when cooking.

`person.bin` starts with the magic `WH04` and seven little-endian u32 counts
(vertices, indices, four identity morph counts and a lid closure count). Each 22-byte vertex holds
position and normal (six i16), UV (two u16), four cumulative limb weights
(four u8), the material (u8) and the total weight of the skull and its child joints (u8). The u16 triangle indices follow the vertices.
Each morph then lists only changed vertices as a u16 index and three signed
i8 displacements. The four identity fields use a range of 0.055 body heights. The lid closure uses 0.012 and occupies the spare fourth components of the first three GPU morph attributes. The cooker uses the pinned `targets/expression/units/caucasian/eye-{left,right}-closure.target` pair for the shared lid motion. The four identity fields store deviations from the mean of four eye-aligned adult reference heads from `targets/macrodetails`. Both renderers blend them with nonnegative weights summing to one. The head weight occupies the fourth morph attribute’s spare component on the GPU. The cooker grows tapered hairline triangles and a recessed crown from the fitted groom outline. The coiled style uses that envelope and a tileable procedural curl pattern in the original atlas slot. The facial sheen mask occupies the existing skin tiles’ alpha.

The material also selects wardrobe visibility. Its low five bits select the surface. Bits five through seven select the outfits that expose the triangle, with zero shared by all. The short-sleeved and jacket outfits share their trousers. The cooker stores the three neutral cloth maps in the RGB channels of one existing atlas tile. Its alpha contains digits and six wordmarks. The renderer combines digits into shirt numbers. Rebuilding the prints uses Pillow 10.1 or newer.
