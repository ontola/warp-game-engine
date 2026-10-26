# Cluster traversal — r46 candidate

Status: implemented behind `KERN_CLUSTER_TRAVERSAL=1`, **off by default and not promoted**. The first native pair preserves exact selected geometry and skips about half the accelerated owners' cluster tests, but shows no net GPU saving. Strict preview pixel parity remains failed. No four-view ABBA is planned for this version. The target remains one 4K output with four players at 120 FPS; no result here establishes that target.

## Work removed

The r45 instance hierarchy can reject whole groups of objects, but every surviving owner still loops through every resident cluster, including all LOD levels. The r46 candidate adds a shared per-mesh hierarchy over those cluster tests. It preserves original owner and cluster IDs, current instance transforms, material rules, wind, and the final projected-error test.

Meshoptimizer's original DAG links are not retained in the packed renderer mesh. This candidate therefore does **not** infer those links or claim to traverse that DAG. It builds a binary bounding hierarchy over the existing cluster predicates. Similar own/parent error ranges are grouped first, with spatial ordering within identical ranges. Pre-order nodes carry escape indices, so traversal needs neither a bounded workgroup stack nor duplicate-child suppression.

Meshes with fewer than 64 resident clusters use the original flat loop. Other valid mesh ranges have leaves of at most 32 clusters. A workgroup traverses one owner with a uniform node loop; retained leaf lanes call the same `cull_cluster` function as the reference path. No source triangles, LOD thresholds, or alpha geometry are changed.

## Conservative rejection

Each node stores enclosing spheres for its descendant cull, parent-error, and own-error bounds, plus the maximum parent error and minimum own error. A node is skipped only if:

- its enclosing cull sphere is outside a camera plane or fully hidden by the existing camera fog policy;
- an upper bound proves every parent error is below the current threshold; or
- a lower bound proves every own error is above that threshold.

Constructor bounds use double-precision accumulation and explicit expansion. The GPU transforms them using an affine operator upper bound, current instance scale, current wind displacement, and roundoff margins. Both sides of the original LOD cut retain the same shear/error scaling. Borderline errors remain with the exact predicate rather than relying on bound equality. Root sentinel parent errors never become ordinary finite parent cuts.

The new node test deliberately adds no group Hi-Z rejection. The existing instance and per-cluster current-frame Hi-Z tests remain unchanged, which keeps strict selected-pair equality a useful oracle. Sun maps, cached pages, split static/dynamic solar maps, and perspective headlights keep their independent flat cluster selection. Camera rejection never removes a shadow caster.

Invalid constructor geometry stays unindexed. Unknown/subset runtime mesh ranges and invalid transforms fall back to the original loop before any accelerated clusters are emitted. Small/unindexed owners and invalid/mismatched fallbacks have separate counters. Scene rebuilding, including tree settings, rebuilds the immutable index. Changing uploaded cluster geometry without rebuilding its metadata is not supported.

## Integration and measurement

Implementation lives in `crates/render/src/cluster_traversal.rs`, `shaders/cluster_traversal.wgsl`, and the shared predicate in `shaders/cull.wgsl`. Renderer integration provides one shared packed read-only buffer at cull binding 12 and one 32-byte counter buffer per view at binding 13. Together with existing scene, spatial, and sun-list buffers, the cull layout uses 15 storage bindings, within the existing limit of 16.

There is no additional GPU pass or per-frame CPU walk. Traversal time is inside the existing camera cull intervals. Metadata construction happens once per renderer/scene. For `C` scene clusters, `N` hierarchy nodes, and `I` indexed cluster references, the shared packed buffer uses `16 + 8*C + 80*N + 4*I` bytes. Each view adds 32 bytes for counters. Disabled mode binds a 16-byte dummy shared buffer. Shared bytes are counted once across views.

Explicit diagnostic capture reports enabled state, shared bytes, indexed meshes, node count, and actual per-view work summed over both camera phases:

- accelerated owners and their resident cluster references;
- tested/rejected hierarchy nodes;
- exact cluster-predicate evaluations and cluster references skipped by rejected nodes;
- small/unindexed flat owners and invalid/mismatched fallback owners.

`clusters_tested + clusters_skipped == resident_clusters` is checked on readback. These are traversal-work counts, not selected triangles, shaded pixels, or unique world objects. Counters reset before the first camera phase and accumulate through the second. Readback remains blocking only on explicit excluded diagnostic captures.

When enabled, stderr also records the 16 most-instanced active mesh ranges, their resident cluster counts, owner-slot counts, and whether they are indexed. This constructor-only inventory verifies whether the actual tree meshes meet the 64-cluster threshold; it must not be inferred from aggregate scene totals.

## Validation scope and remaining risks

Focused tests cover complete deterministic membership and escape links, source immutability, invalid/small ranges, diagnostic accounting, the composed shader/storage budget, and comparison with an independent double-precision flat-selection oracle. The oracle sweeps camera distances, error thresholds, wind, nonuniform scale, rotation, and shear; actual authored fir A/C cluster meshes are included. The coordinated pre-build suites passed **435 renderer tests and 164 game tests**. Actual mesh eligibility and the first GPU pair are recorded below.

The first GPU comparison must retain exact selected `(owner, cluster)` multiset fingerprints, actual camera/lighting/wind state, zero fallback counts, image differences, and the whole renderer GPU interval. New traversal can cost more than a flat scan when most nodes survive: node reads, uniform workgroup barriers, and conservative bounds are overhead. This is particularly relevant to small tree meshes and near cameras. Dedicated meadow rendering and tiny shared plant primitives are not accelerated by this path. No performance improvement is claimed from a reduction in counters alone.

Use the same executable for off/on comparisons, with instance grouping and other independent candidates fixed. Native camera-cut and evolving-wind checks precede an explicit four-view 4K-output comparison. Preserve any failed strict pixel result; do not silently turn a selected-geometry match into image equivalence. No default promotion has been made.

## Native pair result — no promotion

Evidence: `C:/dev/perf/warp-sideways-20261026/r46-work-reduction/clusters-native-recovery-a/report.json`. Both runs use SHA256 `7badc5fdf630c96e6fcc255af37781d3f3defbcb16fc76c5fed38b23e5288675`, one native 1920×1080 view, seed 1/tick 6000, fixed wind, and two captured cameras. Each camera has 128 frames, 32 warmup frames, and **95 clean GPU samples** after capture exclusion. This is one off/on pair, not ABBA or a confidence estimate. The completed off run was retained from `clusters-native-a`; that attempt stopped because of a harness metadata-key error, not a renderer failure.

All four first/settled input and selected-pair fingerprints match exactly. Both near previews are pixel-exact; the two far 1280×720 previews differ at 520/532 pixels, at most 3/255 in a channel, with mean absolute byte differences 0.000195/0.000203. The report correctly retains `passed:false`, `accepted:false`, and `promoted:false`. These previews do not establish full-output pixel equivalence or temporal correctness.

The enabled startup inventory establishes that the dominant source meshes really qualify:

| Tree source | Resident clusters | Owner slots | Indexed |
| --- | ---: | ---: | --- |
| Canopy | 87 | 175,420 | Yes |
| Tapered conifer | 79 | 95,848 | Yes |
| Stout trunk | 63 | 47,911 | No; original flat path |

Thus 271,268 of 319,179 tree instances (84.99%) are eligible. These are constructor owner counts, not visible instances. Across all geometry the index has **38 meshes, 778 nodes, and 333,876 shared bytes** (about 326 KiB), plus 32 bytes per view for counters. Most small terrain ranges remain flat as intended.

The first and settled captures have identical work counts within each camera:

| Actual camera work | Near | Far |
| --- | ---: | ---: |
| Accelerated owners | 16,430 | 12,408 |
| Resident cluster references | 1,386,983 | 1,051,654 |
| Exact cluster tests executed | 704,856 | 534,676 |
| Cluster references skipped | 682,127 (49.18%) | 516,978 (49.16%) |
| Nodes tested / rejected | 84,428 / 17,384 | 64,766 / 13,526 |
| Small/unindexed flat owners | 3,264 | 3,437 |
| Invalid/mismatched fallbacks | 0 | 0 |

Counters accumulate both camera phases; they are not unique objects. Tested plus skipped equals resident in every capture. Reducing those tests did **not** reduce the whole GPU interval:

| Median GPU scope | Near off → on | Far off → on |
| --- | ---: | ---: |
| Renderer interval | 20.185 → 20.327 ms | 19.153 → 19.357 ms |
| Tree occluder cull | 0.034 → 0.161 ms | 0.040 → 0.179 ms |
| Tree remaining cull | 0.296 → 0.300 ms | 0.419 → 0.410 ms |

The whole interval increases by 0.143 ms (0.71%) near and 0.204 ms (1.06%) far in this limited pair. The largest measured cull regression is in the occluder phase. A plausible explanation is that node loads and uniform workgroup barriers cost more than the original few 64-lane iterations for the common 79/87-cluster meshes. This is a hypothesis: combined counters do not isolate the causal cost of each node, barrier, or mesh, and compiler occupancy was not measured. The remaining-phase reduction is small and does not offset the other costs.

CPU wall and cadence are separate, potentially contended measurements; they are not added to GPU intervals or used to infer GPU FPS. Keep this candidate off, retain its diagnostic evidence, and avoid a four-view performance campaign until a substantially cheaper traversal or a more selective workload is demonstrated.
