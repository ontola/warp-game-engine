# Growing Guns character assets

These are converted assets from the existing Growing Guns project, preserving
its active three-character roster and its pistol animation retargeting. No FBX
files or Godot runtime are required by Warp. `zombie.fbx` (the Avelange brute) is
excluded because the original game's roster excludes its incompatible rig.

The input working copy was based on commit
`28cca1617ba0b5e8d0dbb76baf34837c4a6b5f65`, but contained modified model and
animation files. The exact input and output SHA-256 hashes are in
`provenance.json`; those hashes identify the source bytes more precisely than
the commit. The original project describes the characters as Mixamo-rigged.
Its only general project license is MIT (GameNight contributors, 2026), and no
separate model or texture license was found. That software license is **not**
asserted here as a new license for these third-party models, textures, or clips.
Existing upstream rights and any original asset terms remain in force. No
new authorship or redistribution permission is claimed by this conversion.
The source software notice, relevant to the copied animation retargeting code,
is retained in `SOURCE-SOFTWARE-LICENSE.txt`.

The original shader uses albedo textures, roughness 1, metallic 0, and opaque
materials. Paladin and the classic zombie additionally carry their original
normal maps; knight has none. The classic zombie's stored albedo alpha is 0.8
but transparency is disabled in the source; it remains opaque here. Texture
PNG bytes are unchanged. Material albedo values are stored in sRGB, matching
Godot's [`source_color` albedo uniform](https://github.com/godotengine/godot/blob/4.6-stable/scene/resources/material.cpp#L905),
and must be converted to linear before use as Warp material tint.

## Regenerate

Run the offline exporter with an installed Godot 4.6.2 executable and a source
Growing Guns checkout. The script creates an isolated import project under
`build/character-extract`, never starts the source game, and does not edit it.

```powershell
games/growing-guns/tools/export-characters.ps1 `
  -SourceRoot '\\wsl.localhost\Ubuntu\home\joep\dev\playtest\growing-guns' `
  -Godot 'C:/dev/tools/godot-4.6.2/Godot_v4.6.2-stable_win64_console.exe'
python games/growing-guns/tools/validate-characters.py
```

The validator uses NumPy. `--preview path.png` additionally uses Pillow and
creates a CPU-rendered contact sheet without a GPU context.

## Format version 1

Each character has a JSON manifest, `NAME.mesh.bin`, and `NAME.anim.bin`.
Binary numbers are little-endian. Mesh data contains `vertex_count` consecutive
96-byte vertices, followed by `index_count` global `u32` indices:

| Byte offset | Field |
| --- | --- |
| 0 | position, `f32[3]` |
| 12 | unit normal, `f32[3]` |
| 24 | UV, `f32[2]` |
| 32 | palette joints, `u32[8]` |
| 64 | normalized weights, `f32[8]` |

Unused influences have zero weight. Surface ranges partition the vertices and
indices; indices already address the entire character vertex buffer. Winding
is counter-clockwise, converted from Godot's clockwise imported triangles.
UVs retain Godot's top-left texture origin and the texture images are sRGB.
Normals and vertices are transformed into the same model coordinate frame.

Animation is sampled at 30 fps. At each clip's `matrix_offset_bytes`, consecutive
frames contain `joint_count` matrices. Each matrix is twelve `f32` values:
three rows of a 3x4 affine matrix, with translation in the fourth column.
The implicit fourth row is `[0, 0, 0, 1]`. These are **final skin matrices**,
mapping an exported bind vertex to its animated model position; do not apply
an inverse bind again. Palettes contain only used bindings. They retain all
four influences for knight/paladin and all eight for the classic zombie.

Looped idle/run contain 47/22 frames without duplicate endpoints. One-shot jump
contains 61 frames including the final endpoint, so its sample coordinate is
`time/duration*(frames-1)`. Loops use `phase*frames` and wrap the next sample.
Linear matrix interpolation matches the renderer's bounded pose blending.
Clip bounds are the union of every weighted vertex at every exported sample;
matrix interpolation and convex clip blending remain within their union.

The coordinate system is right-handed, Y up, metres, model forward +Z. The
game applies its existing 180-degree Y rotation and -0.9-metre foot offset.
Neither is baked into the asset. The original retargeting intentionally drops
the knight-only root translation for paladin/zombie. Consequently only knight
retains the jump clip's vertical root excursion, matching the source behavior.

Per-frame `sockets` hold original model-space Head, RightHand, and Hips joint
transforms in the same 3x4 format. They already include the skeleton frame and
are intended for head crests and weapon attachment; they are not skin matrices.
