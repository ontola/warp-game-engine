# Correcting the proportions of seeded faces

On 11 November 2026 on Mancave (Windows, Vulkan, RTX 5070 Ti), we compared the released close faces from `22d960ef` with the correction based on `fafc1fc6`. The fixtures used Growing Guns, full graphics, arena 13, noon, two views at a combined 2560 × 1440, the second player and match seeds 23, 11, 45, 22, 52 and 873. The face review places the camera in front, at three quarters and near profile. A fourth view has the player aim upwards.

Several signed targets for the nose had shared one random coefficient: a negative draw reduced its depth and rotated its tip upwards together. The adult target difference also included the difference in vertical position of the entire head between source bodies. Jaw breadth changed again during fitting. Finally, the player binding assigned the lower face to the neck by height, so the jaw could move against the nose during a turn.

The cooker now registers each complete adult reference at the midpoint of its eyes, blends the neck into the body and stores deviations from the mean. The four weights are nonnegative and sum to one. Fitting compensates for body breadth at the skull. The source skeleton's skull subtree determines how much of the neck rotation reaches each vertex. Its quantized weight fits the high byte of the cooked surface word and the spare fourth component of the fourth identity attribute.

The measurements below use selected corresponding hm08 vertices on the 1.80 m fitted player in the bind pose. They describe this asset, rather than population norms. The cheek measure is the difference between the two widths, computed by doubling the difference between lateral landmarks on the same side.

| Match seed | Landmark measurement | Previous | Corrected |
|---|---|---:|---:|
| 23 | Forward projection, nasal tip from nasal base (vertices 297 and 317) | 3.7 mm | 12.6 mm |
| 23 | Vertical rise between the same two vertices | 22.0 mm | 13.3 mm |
| 11 | Cheek width minus jaw width (7572 and 7613) | 19.4 mm | 13.4 mm |
| 45 | Vertical distance from lower lip to chin (495 and 747) | 29.0 mm | 42.7 mm |

The cooked mesh grew from 1,234,007 to 1,434,610 bytes because the four fields cover complete heads. The atlas occupies 2,290,219 bytes. Together they occupy 3,724,829 bytes on disk. The spectator record stays at 64 bytes and each packed mesh vertex at 44 bytes.

All six match fixtures completed and all three stadium reviews drew the streamed close mesh. Six renderer and ten player/locomotion tests passed. A second cook reproduced both shipped assets byte for byte.
