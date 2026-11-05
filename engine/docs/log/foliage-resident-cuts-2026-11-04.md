# Resident intermediate vegetation cuts

Normal shared selection now gates refinement on residency of the visible
immediate child representations. If any child is unavailable, it requests all
missing siblings and retains the current resident node. Once the child bundle
is available, each child can independently refine or retain its own ancestor.
This stops one missing detail page from replacing an otherwise loaded whole
tree. The conservative root fallback still handles work/record/slot overflow.

The same rule runs in demand counting and publication, and in the standalone
history traversal. Residency is immutable during those dispatches. Therefore
budget admission counts the actual publishable cut, and no parent is emitted
alongside part of its child replacement. Frustum-invisible children do not
request pages. Camera/shadow union visibility still determines the needed cut;
a camera cannot discard another consumer's caster. Forced-refinement nodes may
be used as emergency resident fallback, even if they fail the optical quality
policy, just as the pinned root was previously used for asset faults.

Requests include the intermediate visible path. This increases the working set
needed to progress to fine detail and may need a future cut-aware cache admission
policy under severe pressure. Physical budgets remain bounded. This change does
not tighten the overly broad root mip pins, implement content edits, or solve
history-first upstream selection and animated shadow-page caching.

## Evidence

Release compilation and Naga shader validation pass. The new raw-buffer selector
fixture passes using Windows DX12 software rendering (KERN_ADAPTER=software,
WGPU_BACKEND=dx12). Four consumers share a two-branch tree: with one missing leaf,
only its immediate branch ancestor is emitted; the other branch retains leaves.
Loading restores the fine cut, evicting an intermediate parent retains the root,
reloading recovers, and a two-slot budget rejects the three-record cut completely.
The missing page appears in compact GPU feedback.

The existing shared-pose matrix/hash-overflow oracle also passes on that software
adapter, including the previous indirect-depth dispatch change and zero-work
arguments after an asset miss. The normal cold-start renderer fixture has been
updated to require a complete direct ancestor cut instead of a whole-tree error
flag while source assets load. Its execution and physical-GPU confirmation are
recorded in HANDOFF. Software-adapter timing is not a performance result.

Additional software fixtures pass: wavefront/serial cut equivalence with bounded
queue overflow, and shared union budget priorities. The larger standalone
historical disocclusion fixture hit its 30-second poll timeout and is not counted
as passed. Full renderer cold-start on software was deliberately stopped above
17 GB working set; hardware execution remains required. No new visual or frame
performance acceptance is claimed.
