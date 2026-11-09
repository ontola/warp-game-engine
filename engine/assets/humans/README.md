# Warp Humans close character sources

The reference body, fitting data and textures derive from MakeHuman hm08 and
the MakeHuman system asset pack, released under CC0. The source copyrights
identify Data Collection AB, Joel Palmius and Jonas Hauquier.

- [hm08 mesh, deformation targets and skinning weights](https://github.com/makehumancommunity/makehuman/tree/a8bc2d54ff0ac92e78ff71431b1023eda42bf482/makehuman/data)
- [MakeHuman system pack, asset list and licences](https://static.makehumancommunity.org/assets/assetpacks/makehuman_system_assets.html)

The cooker uses base.obj, default_weights.mhw and the targets listed in
`engine/tools/cook-humans.py`. Place the weight and target files together.
Extract male_casualsuit01, shoes02, eyebrow001, short01, bob02, afro01 and the
three listed skin materials into the system pack's original directory layout.
The cooker fits clothing and groom vertices from the mhclo barycentric bindings
and transfers their skin weights and facial displacements from the same body.

```sh
python engine/tools/cook-humans.py base.obj source-files/ system-pack/ engine/assets/humans/
```

person.bin contains two little-endian u32 counts, 38-byte bind-pose vertices
and u32 triangle indices. Each vertex stores position and normal (six i16),
UV (two u16), four cumulative joint weights (four u8), four signed XYZ morphs
padded to four bytes each, and a material identifier (u16). appearance.png
contains skin, cutout hair, clothing and brow tiles.
