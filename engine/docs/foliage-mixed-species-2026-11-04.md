# Mixed species and grass assembly admission

This is shared-runtime infrastructure, **not completion of the meadow migration**.
The terrain demo and games still use their existing meadow placement/render path.
No new backend flag or second grass renderer was introduced.

`foliage_runtime::Species` pairs a borrowed CookedPlant with GeometryCategory.
`plant_species_population` attaches mixed catalogs to the same nodes, branch slots,
logical asset pages, proxy fallback, pose scheduling and shadows used by trees.
Existing plant/plant_population APIs forward to this path with Trees categories.
Stable WorldRoot variant indices select a species; replacing a species on an
existing root changes generation while retaining unrelated root identities.

SceneVegetation previously keyed virtual member templates only by the CookedPlant
address. Reusing that asset under another category silently inherited the first
category in composed members. The template key now also includes category. These
are compact transform/flag records; source meshes and voxel assets stay shared.

`recipe_forest::cook_meadow_variants` cooks the existing 120-blade ribbon patch
into CookedPlant branch aggregates and a five-level whole-patch voxel fallback.
It uses the existing on-disk cook cache and geometry-derived optical evidence.
Four moving blade groups retain their articulated parent root. No fake trunk or
extra source member is inserted to satisfy a tree-shaped hierarchy.

A real grass cook initially failed because cook_aggregate required source members
on the leading root. That restriction is removed: a pure articulation root gets
a whole-subtree aggregate; exact source leaves are generated only for branches
with real members. Existing root work/memory caps remain enforced. The regression
checks that no phantom root SourceBranch is published and coarse fallback remains.
Scene binding validation accepts that empty root only as an aggregate parent with
child groups and a matching root articulation/volume owner. Ordinary empty source
bindings still fail. Exact selection expands the children; it does not draw an
invented source root.

The aggregate cooker also distinguished only closed components. It now follows
Assembly's explicit projection measure: closed surfaces contribute half their
absolute projected area (entry/exit pairs); open meadow ribbons contribute the
full area (one crossing). Source reference quadrature, triangle alternatives and
all reduced voxel levels use the same measure. Both branch and whole-plant assets
retain that metadata. Closed-topology validation remains mandatory for closed
species; sheet assets do not falsely report closed-part validation.

A known-area open-triangle regression checks all 13 directional masses across
branch and whole-plant levels, and verifies that closed mode rejects the same
open geometry. This tests optical mass, not final image equivalence.

## Spatial cell authoring

`vegetation_cell::compose` combines repeated source assemblies under one cell
root, sharing the source geometry and material palette. It copies compact branch
and placement records, not expanded vertices. Input member IDs determine stable
ordering; their rigid transforms carry local height/orientation. Motionless source
roots merge into the cell root, without introducing extra moving ancestors.
Output uses the same CookedPlant source/aggregate/proxy compiler and population.

The CPU oracle compares every transformed source vertex/normal, checks bounded
rejection and order-independent identities, then cooks and attaches a two-patch
cell (240 blades, nine shape/palette meshes, nine branches) as one world root and
one proxy. This is authoring infrastructure, NOT running world-cell streaming.
Rigid patch placement does not conform individual blades to curved ground; the
existing basal-wind and terrain-attachment requirements still apply. Extreme
range cell hierarchies and mixed-species optical measures remain future work.

## Remaining migration requirements

- Prove the mixed grass/tree materialization and draw path on GPU, including wind,
  category exclusions, missing pages and population replacement.
- Replace meadow placement with stable world cells using the same root transactions.
  Do not blindly replace every current procedural patch with a permanent CPU-built
  record over the full terrain radius: that would increase root memory/traversal.
- Share patch primitives inside spatial cell assemblies, then use cell aggregates
  at distance instead of per-patch records.
- Preserve blade basal anchors under wind: the current common articulation rotates
  four groups about their pivots, whereas the meadow path bends each blade about
  its own base. A successful cook is not proof of equivalent grass animation.
- Preserve terrain/road exclusion and ground conformance. RootPlacement currently
  supports yaw and uniform scale; broad rigid grass cells alone would float above
  or penetrate slopes, so ground attachment needs an explicit shared contract.
- Validate source/aggregate coverage, wind bounds, shadows and measured four-view
  GPU timings before claiming the old meadow path has been replaced.

Tests described in HANDOFF are CPU cook/attachment/identity checks unless explicitly
marked GPU. They do not demonstrate final meadow appearance or a performance win.
