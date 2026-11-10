# Warp Humans close character sources

The reference body, fitting data and textures derive from MakeHuman hm08 and
the MakeHuman system asset pack, released under CC0. The source copyrights
identify Data Collection AB, Joel Palmius and Jonas Hauquier.

- [hm08 mesh, deformation targets and skinning weights](https://github.com/makehumancommunity/makehuman/tree/a8bc2d54ff0ac92e78ff71431b1023eda42bf482/makehuman/data)
- [MakeHuman system pack, asset list and licences](https://static.makehumancommunity.org/assets/assetpacks/makehuman_system_assets.html)

The cooker uses base.obj, default_weights.mhw and the targets listed in
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

`person.bin` starts with the magic `WH02` and six little-endian u32 counts
(vertices, indices, then four sparse morph counts). Each 22-byte vertex holds
position and normal (six i16), UV (two u16), four cumulative limb weights
(four u8) and the material (u16). The u16 triangle indices follow the vertices.
Each morph then lists only changed vertices as a u16 index and three signed
i8 displacements. The loader expands these once into the crowd's GPU layout.

The material also selects wardrobe visibility. Its low five bits select the surface. The upper bits select the outfits that expose the triangle, with zero shared by all. The short-sleeved and jacket outfits share their trousers. The cooker stores the three neutral cloth maps in the RGB channels of one existing atlas tile. Its alpha contains digits and six wordmarks. The renderer combines digits into shirt numbers. Rebuilding the prints uses Pillow 10.1 or newer.
