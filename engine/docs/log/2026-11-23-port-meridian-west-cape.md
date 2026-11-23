# Port Meridian's western cape

Game revision `08827804` adds 474,786.5 m² of traversable land to the 126,736 m² city, bringing the combined area to 0.602 km². East to west it spans 1,388 m. A 152 m connector leads to a 1,679.97 m paved loop. Generated coast sections and road centerlines drive the mesh, world queries and per-seat minimap. The cape uses a texture-free glTF stand baked by the game, rendered by Warp’s general mesh cluster system. The original 27 city pines use the Luau Warp Vegetation preview, and the two city lawns use Warp’s GPU meadow. The woodland timing below measures the glTF path. The stand contains fifteen distinct stems and fern clumps. The playtest seed (1948) places 48 stands, or 720 trees, across two groves.

## GPU visibility

The native headless renderer ran at 1600 × 900 on an NVIDIA RTX 5070 Ti. The probe read the source instance and cluster index from each camera's visibility buffer via `Renderer::debug_visible_clusters` and used `Renderer::debug_sun_culling` for the same rendered frame. Each selected cluster was mapped back to its instance and its triangle count. The sun's selection was read independently, including after an explicit shadow refresh. These counts represent the pieces submitted for rasterization.

| Location and viewing direction | Woodland stands | Woodland clusters | Woodland triangles |
| --- | ---: | ---: | ---: |
| Downtown, looking down Exchange Avenue | 0 | 0 | 0 |
| Woodland edge, looking into the trees | 48 | 3,453 | 263,758 |
| Same point, facing the other way | 0 | 0 | 0 |
| Helicopter at 75 m, looking into the trees | 48 | 3,172 | 236,006 |
| Helicopter at the same point, facing away | 0 | 0 | 0 |
| Two players, forest player's view | 48 | 2,841 | 192,586 |
| Two players, simultaneous downtown view | 0 | 0 | 0 |

The woodland-edge camera was at (-544, 2, -99). The shared sun pass selected 32 stands and 2,792 clusters facing either way. In the solo downtown sample the woodland contributed zero clusters to both sunlight passes. The cape terrain is one mesh with a cluster hierarchy. In the downtown camera 12 clusters and 613 triangles remained, compared with 27 clusters and 1,678 triangles facing the tree line.

At the city end of the cape, the same camera at (-220, 16, 0) selected 17 vehicles, nine street trees, nine shrub beds and 202 scenery objects facing downtown. Turning 180 degrees selected zero vehicles, street trees and shrub beds, and five scenery objects. In split screen the forest view selected zero cars while the downtown view selected nine.

## Frame cost

GPU hardware timestamps compare the same landscape with all 48 stand meshes visible and hidden. Terrain, roads, collision, simulation and sunlight were identical. Each paired run warmed up for 105 frames and sampled 120 frames in each configuration. The table averages the two run medians and p95 values. The view on the road used (-673, 1.85, -129), the western grove used (-894, 1.85, 110), and the helicopter view used (-567, 75, -13).

| View | GPU median, stands hidden | GPU median, stands visible | Difference | GPU p95, stands visible |
| --- | ---: | ---: | ---: | ---: |
| Road through Mariner Wood | 1.631 ms | 1.804 ms | +0.173 ms | 2.035 ms |
| Among the trees in the western grove | 1.694 ms | 1.902 ms | +0.208 ms | 2.147 ms |
| Helicopter over the cape | 1.531 ms | 1.806 ms | +0.275 ms | 1.856 ms |

The populated downtown view took 2.496 ms median on the new island and 2.605 ms in the preceding city-only build. In a shared 1600 × 900 frame the forest eye took 0.710 ms, the downtown eye 1.101 ms and shared work 0.875 ms with the woods enabled.

The end-to-end benchmark waits for the GPU each frame. Its median in flight went from 9.95 to 11.24 ms. The two ground-level medians changed by at most 0.22 ms.

## Simulation and navigation

Cars keep all fixed trunk footprints in Rapier's spatial index. Local players share 24 m collision cells for walking, flight and camera collision. The cells around a grounded player become active on a cell crossing and are released when the last player leaves, including climbing above the canopy. A distant scripted walker can test its next step against the same stored footprints.

The native controller crossed the old city limit on foot and in a car. A second driving test reached the outer bend at x = −1,076.5 m. A helicopter crossed the seam, landed on the cape and let its pilot get out. The trunk test stopped the walker at the expected capsule radius and the clear road allowed continued walking. The HUD tests exercised 216 combinations of position, travel mode, goal bearing and viewport layout. Custom checkpoints on the cape and rejection of offshore points are covered by the game test suite.

Across 4,096 sampled layout seeds, initialization peaked at 1,598 world entities and generated 45–51 stands (675–765 trees). In 24 full four-seat simulations, placing four walkers in their densest separate cell neighborhoods activated up to 405 trunk bodies. Across the room/woodland combinations the largest runtime count was 1,518 before temporary effects. The game reserves 1,800. In the controller test, the capsule slid around a trunk corner at 0.320 m clearance and stopped 0.480 m inside the inclined cliff, matching its radius plus half the width of the guard.

Revision `f6167a61` streams against the positions from the current tick. It releases stale rooms and trunk cells before allocating new ones. Direct Luau placement of a four-player party between four dense forest neighborhoods and four offices passed in both directions, on fourteen seeds, using two placement styles (56 transitions). In seed 53154, the first indoor frame had four rooms and zero trunks; the return frame had zero rooms and 395 trunks. Nonconsecutive player IDs shared the same cells. Flight at 35 m freed all 53 trunk colliders previously active beside a sampled tree.

## Web transfer

The WebGPU shelf `08827804-20261123101505` reuses the same engine binary. Edge transferred 19,679,476 bytes on a cold visit, versus 19,630,262 on the directly preceding shelf (49,214 additional bytes, 0.25%). A browser primed on the previous version transferred 119,636 bytes for the update including response headers and zero engine bytes. Reopening the same version transferred 273 bytes. The two new model files are 71,724 and 68,828 bytes before compression. Each is shared by all players and generated placements. All 84 public file hashes were verified.

The release with the streaming-order fix, `f6167a61-20261123102111`, changes four Luau files. Public cold transfer was 19,679,987 bytes. Updating from the previous cape build transferred 51,223 bytes and a repeat visit 250 bytes. A public profile primed with the 50 matching cache keys observed in the earlier city-only release transferred 132,362 bytes. An exact city-only to final same-origin replay transferred 133,247 bytes. All returning profiles reused the engine. Edge verified all 55 decoded bodies. Public WebGPU also rendered the city, both groves and the 75 m flight view and crossed the old city boundary on keyboard input.
