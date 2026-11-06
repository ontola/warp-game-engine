// Cluster culling and LOD selection, one workgroup per instance.
// Foliage seed view marker in fog_cull.w (above the category bits), and the
// instance (scene FOLIAGE|LIVE_GRASS|WIND_GRASS|PART_WIND|SKINNED) and
// material (ALPHA_TEST|GRASS|BLEND|THIN_FOLIAGE) flags it refuses. Not 128:
// MOLTEN_DEBRIS (rigid, so a fine occluder) shares it with procedural terrain
// PROCEDURAL, and the land must always occlude.
const OCCLUDERS_ONLY:u32=128u;
const DEFORMING:u32=1u|8u|16u|32u|64u;
const SEE_THROUGH:u32=1u|64u|256u|512u;
// A cluster is drawn when its own error projects to at most the threshold
// and its group's simplified replacement would project above it.
// Adreno 650 misreports OpArrayLength for struct arrays (four 80-byte
// instances report two). The renderer already fixes this count at construction.
override INSTANCE_COUNT: u32 = 0u;
override CLUSTER_BUCKETS: bool = true;
// Opaque clusters draw first through a pipeline without `discard` (early
// depth); alpha-tested ones after it. Needs CLUSTER_BUCKETS.
override RASTER_SPLIT: bool = false;
override EARLY_GRASS_CULL: bool = true;
// Scene::voxel_far_distance_m; zero when the scene has no far trees.
override VOXEL_FAR_M: f32 = 0.0;

struct View {
    view_proj: mat4x4<f32>,
    inv_view_proj: mat4x4<f32>,
    cam: vec4<f32>,          // position; w: pixels per unit error at distance 1 (or per metre, ortho)
    planes: array<vec4<f32>, 6>,
    params: vec4<f32>,       // threshold px, znear, ortho, max visible
    size: vec4<f32>,
    extra: vec4<f32>,        // x: alpha test threshold
    grass: vec4<f32>,        // source-grass fade start/end, shared with residency
    fog_cull: vec4<f32>,     // camera: fog distance, cull enable, sun visibility, category exclusion; sun views: z/w caster LOD diagnostics
};

struct Cluster {
    cull: vec4<f32>,
    parent: vec4<f32>,
    own: vec4<f32>,
    parent_error: f32,
    own_error: f32,
    first_tri: u32,
    tri_count: u32,
    first_vert: u32,
    material: u32,
    depth: u32,
    pad: u32,
};

struct Instance {
    r0: vec4<f32>,
    r1: vec4<f32>,
    r2: vec4<f32>,
    sphere: vec4<f32>,
    first_cluster: u32,
    cluster_count: u32,
    scale: f32,
    flags: u32,
};

struct CullMaterial {
    tint: vec3<f32>,
    layer: i32,
    roughness: f32,
    flags: u32,
    scale: f32,
    metal: f32,
};

@group(0) @binding(3) var<storage, read> clusters: array<Cluster>;
@group(0) @binding(4) var<storage, read> instances: array<Instance>;

// Address/deformation composition is shared in virtual_member.wgsl.
fn instance_at(id: u32) -> Instance {
    if (id & VIRTUAL_MEMBER) == 0u { return instances[id]; }
    let owner = instances[member_owner(id)];
    return compose_member(owner, instances[member_template(owner,id)]);
}
// Materials and crown replacement words from the scene tables
// (scene_tables.rs): materials from vec4 0.x, two vec4s each; replacement
// words from vec4 2.z.
@group(0) @binding(5) var<storage, read> scene_tables: array<vec4<u32>>;
fn cull_material(i: u32) -> CullMaterial {
    let at = scene_tables[0].x + 2u * i;
    let a = scene_tables[at];
    let b = scene_tables[at + 1u];
    return CullMaterial(bitcast<vec3<f32>>(a.xyz), bitcast<i32>(a.w), bitcast<f32>(b.x), b.y, bitcast<f32>(b.z), bitcast<f32>(b.w));
}
fn cull_voxel_replacement_at(i: u32) -> u32 {
    let v = scene_tables[scene_tables[2].z + (i >> 2u)];
    return v[i & 3u];
}
@group(0) @binding(12) var<storage, read> cull_wind: array<vec4<f32>>;

@group(1) @binding(0) var<uniform> view: View;
@group(1) @binding(1) var<storage, read_write> visible: array<vec2<u32>>;
// 0: candidate count, 1: retained triangles, 2: submitted vertices,
// 3: early grass rejects; 4..7: bucket counts, 8..11: scatter cursors.
// 12: phase2 prefix, 13/14: submitted phase1/2 clusters,
// 15/16: tested/rejected owners, 17/18: tested/rejected clusters. 19 reserved.
// 20/21: spatial parents tested/rejected,22/23 leaves tested/rejected,
// 24/25: spatial owner tests/compacted,26 resident cluster loop visits,
// 27 full-scan fallbacks,28 phase-local early-grass scratch,
// 29/30 phase-local expected/visited owners,31 traversal-mismatch fallbacks.
// 32..39: cluster traversal. 40..43: masked clusters per bucket (included in
// 4..7), 44..47: masked scatter cursors.
// The counters, the draw args and the bucket draw map share one binding
// (frame.rs ViewTargets::counter): counter at byte 0, args at 192, the map
// from 512.
struct CullDraw {
    counter: array<atomic<u32>, 48>,
    args: array<u32, 40>,
    pad: array<u32, 40>,
    draw_map: array<u32>,
};
@group(1) @binding(2) var<storage, read_write> cull_draw: CullDraw;
// Four draw records followed by four bucket starts (opaque clusters when
// RASTER_SPLIT, else all), then the same for masked clusters.
@group(1) @binding(5) var<storage,read> foliage_selection_words: array<u32>;
fn foliage_selection_word(i:u32)->u32 {return foliage_selection_words[i];}
fn foliage_selection_at(i:u32)->vec4<u32> {return selection_unpack(foliage_selection_word(i));}
// Packed group selection (aggregate_lod.wgsl writes it): one u32 per group,
// runtime-sized storage table. Bits 0-1 choice, 2-7
// voxel level (63: none), 8-9 sun mode, 16-31 the triangle error as f16.
fn selection_unpack(w:u32)->vec4<u32> {
    let y=(w>>2u)&63u;
    return vec4<u32>(w&3u,select(y,0xffffffffu,y==63u),bitcast<u32>(unpack2x16float(w>>16u).x),(w>>8u)&3u);
}

@group(1) @binding(6) var<uniform> tree_phase: vec4<f32>; // x: 0 reference, 1 occluders, 2 remainder; y split metres; z enables Hi-Z rejection
@group(1) @binding(15) var<uniform> tree_history:array<vec4<u32>,4096>;
@group(1) @binding(7) var tree_hiz: texture_2d<f32>;
// Original source-owner IDs, independent of camera visibility. Header:
// static count, dynamic count, static word offset, dynamic word offset.
struct SpatialGroup {sphere:vec4<f32>,span:vec4<u32>}
struct SpatialGroups {header:vec4<u32>,groups:array<SpatialGroup>}
// The BVH, spatial groups, their members and the sun owner lists, copied
// into one buffer (cull_tables.rs); vec4 0 says where each starts, vec4
// 1.x how many member words there are.
@group(1) @binding(8) var<storage,read> cull_tables: array<vec4<u32>>;
fn cull_table_word(at: u32) -> u32 { let v = cull_tables[at >> 2u]; return v[at & 3u]; }
fn cluster_tree_at(i: u32) -> u32 { return cull_table_word(cull_tables[0].x + i); }
fn spatial_member(i: u32) -> u32 { return cull_table_word(cull_tables[0].y + i); }
fn spatial_members_len() -> u32 { return cull_tables[1].x; }
fn sun_owner(i: u32) -> u32 { return cull_table_word(cull_tables[0].z + i); }
fn spatial_header() -> vec4<u32> { return cull_tables[cull_tables[0].w]; }
fn spatial_group(i: u32) -> SpatialGroup {
    let at = cull_tables[0].w + 1u + 2u * i;
    return SpatialGroup(bitcast<vec4<f32>>(cull_tables[at]), cull_tables[at + 1u]);
}
// Dedicated scratch: phase2 must not overwrite phase1's bucket draw map.
@group(1) @binding(10) var<storage,read_write> spatial_owners:array<u32>;

fn tree_occluded(root:vec3<f32>,radius:f32)->bool {
    // Phase-order diagnostic: preserve both draws and their visible IDs,
    // but remove Hi-Z rejection as a cause of any changed depth winner.
    if tree_phase.z<0.5 {return false;}
    let size=textureDimensions(tree_hiz,0);
    if any(size==vec2<u32>(0u)) {return false;}
    var lo=vec2<f32>(1e30);var hi=vec2<f32>(-1e30);var nearest=0.0;
    // The root-centred AABB encloses the entire source plus its maximum wind
    // envelope. Positive W makes projected extrema occur at its eight corners.
    // A near-plane crossing or any invalid projection is conservatively kept.
    for(var corner=0u;corner<8u;corner++) {
        let sign3=vec3<f32>(select(-1.0,1.0,(corner&1u)!=0u),
            select(-1.0,1.0,(corner&2u)!=0u),select(-1.0,1.0,(corner&4u)!=0u));
        let clip=view.view_proj*vec4<f32>(root+sign3*radius,1.0);
        if !all(abs(clip)<vec4<f32>(1e30)) || clip.w<=1e-5 ||
            clip.z<0.0 || clip.z>=clip.w-1e-6 {return false;}
        let ndc=clip.xyz/clip.w;
        let uv=ndc.xy*vec2<f32>(0.5,-0.5)+0.5;
        lo=min(lo,uv);hi=max(hi,uv);nearest=max(nearest,ndc.z);
    }
    if any(hi<vec2<f32>(0.0)) || any(lo>vec2<f32>(1.0)) {return false;}
    // Include a full base pixel and roundoff allowance beyond both edges.
    // Work in base pixel indices: uv*mipDimensions is NOT equivalent for odd
    // sizes. Reduction folds an odd final source row/column into the last
    // destination texel; integer shifts plus clamp preserve that footprint.
    let last=vec2<f32>(size-vec2<u32>(1u));
    let first_pixel=vec2<u32>(clamp(floor(lo*vec2<f32>(size)-1.001),vec2<f32>(0.0),last));
    let last_pixel=vec2<u32>(clamp(ceil(hi*vec2<f32>(size)+1.001),vec2<f32>(0.0),last));
    let levels=textureNumLevels(tree_hiz);
    // A footprint spanning N pixels cannot fit two texels below
    // floor(log2(N))-1. Skip those provably unusable levels, preserving the
    // exact first accepted mip (including odd-dimension edge folding).
    let span=max(last_pixel.x-first_pixel.x,last_pixel.y-first_pixel.y)+1u;
    let start=select(0u,firstLeadingBit(span)-1u,span>1u);
    for(var level=start;level<levels;level++) {
        let dimensions=textureDimensions(tree_hiz,i32(level));
        let end=dimensions-vec2<u32>(1u);
        let first=min(first_pixel>>vec2<u32>(level),end);
        let last=min(last_pixel>>vec2<u32>(level),end);
        if any(last-first>vec2<u32>(1u)) {continue;}
        // All covered mip texels are tested, including zero-depth openings.
        // A single centre/corner depth tap cannot certify full occlusion.
        let a=textureLoad(tree_hiz,vec2<i32>(first),i32(level)).r;
        let b=textureLoad(tree_hiz,vec2<i32>(i32(last.x),i32(first.y)),i32(level)).r;
        let c=textureLoad(tree_hiz,vec2<i32>(i32(first.x),i32(last.y)),i32(level)).r;
        let d=textureLoad(tree_hiz,vec2<i32>(last),i32(level)).r;
        let depth=min(min(a,b),min(c,d));
        return depth>nearest+2e-5 && depth<=1.0;
    }
    return false;
}

override SMALL_CLUSTER_TRIANGLES:u32=8u;
fn cluster_bucket(triangles: u32) -> u32 {
    if triangles <= SMALL_CLUSTER_TRIANGLES { return 0u; }
    if triangles <= 32u { return 1u; }
    if triangles <= 64u { return 2u; }
    return 3u;
}
// Same test as raster.wgsl alpha_cut: an alpha-tested texture or a fading
// grass card can discard. The game mask disables RASTER_SPLIT on the CPU.
fn masked_cluster(flags:u32, material:u32) -> bool {
    let m=cull_material(material);
    return ((m.flags&1u)!=0u && m.layer>=0) || (flags&8u)!=0u;
}
fn count_cluster(flags:u32, cl:Cluster) {
    let bucket=cluster_bucket(cl.tri_count);
    atomicAdd(&cull_draw.counter[4u+bucket],1u);
    if RASTER_SPLIT && masked_cluster(flags,cl.material) {atomicAdd(&cull_draw.counter[40u+bucket],1u);}
}
fn bucket_vertices(bucket: u32) -> u32 {
    return array<u32, 4>(SMALL_CLUSTER_TRIANGLES*3u, 96u, 192u, 384u)[bucket];
}

fn in_frustum(c: vec3<f32>, r: f32) -> bool {
    for (var i = 0u; i < 6u; i++) {
        if dot(view.planes[i].xyz, c) + view.planes[i].w < -r {
            return false;
        }
    }
    return true;
}

fn hidden_by_camera_fog(c: vec3<f32>, r: f32) -> bool {
    // Only the colour view opts in. Shadow/GI/reflection caster lists stay
    // independent, including trees completely hidden from this camera.
    if view.fog_cull.y < 0.5 || view.params.z > 0.5 { return false; }
    return distance(c, view.cam.xyz) > view.fog_cull.x + max(r, 0.0);
}

fn to_world(inst: Instance, p: vec3<f32>) -> vec3<f32> {
    let q = vec4<f32>(p, 1.0);
    return vec3<f32>(dot(inst.r0, q), dot(inst.r1, q), dot(inst.r2, q));
}

// Screen-space size in pixels of `err` metres at the sphere's nearest point.
fn projected(err: f32, c: vec3<f32>, r: f32) -> f32 {
    if view.params.z > 0.5 {
        return err * view.cam.w;
    }
    let d = max(distance(c, view.cam.xyz) - r, view.params.y);
    return err / d * view.cam.w;
}

// The optional one-thread-per-owner broadphase uses exactly the same policy
// as the legacy workgroup path: 0 rejected, 1 candidate, 2 early grass reject.
fn instance_policy(id: u32, inst: Instance) -> u32 {
    // Only this spotlight excludes its emitter body. Sun and other lamps
    // still see it, including remote players with no local camera view.
    if view.extra.y>1.5 && view.grass.w>0.0 {
        let first=u32(view.grass.z); let count=u32(view.grass.w);
        if id>=first && id-first<count {return 0u;}
    }
    // Unoccupied fixed-pool slots have no geometry. This is uniform across
    // the workgroup, so skip the frustum test/barrier without divergence.
    // A template draws only through its virtual branch owners.
    if inst.sphere.w < 0.0 { return 0u; }
    if inst.cluster_count == 0u && !virtual_owner(inst) {
        return 0u;
    }
    // Scalable whole-owner replacement is a colour-only decision. It neither
    // changes source wind tags nor removes any sun/headlight shadow caster.
    if view.params.z<0.5 && view.extra.y<1.5 && (spatial_header().w&8u)!=0u {
        if source_replacement_owner_selected(spatial_header().z,member_slot(id)) {return 0u;}
    }
    if tree_phase.x>0.5 && tree_phase.w<0.5 {
        let tree=((inst.flags>>24u)&7u)==2u;
        var first=!tree || distance(inst.sphere.xyz,view.cam.xyz)
            -(inst.sphere.w+cull_wind[member_slot(id)].z)<=tree_phase.y;
        if (tree_phase.x<1.5)!=first {return 0u;}
    }
    // Explicit colour-only category ablation. Orthographic sun/pages and
    // perspective headlights keep all casters, as do the retained GI fields.
    if view.params.z<0.5 && view.extra.y<1.5 && view.fog_cull.w>0.0 {
        let code=(inst.flags>>24u)&7u;
        let category=select(code,0u,code>5u);
        if (u32(view.fog_cull.w)&(1u<<category))!=0u {return 0u;}
        // Foliage seed (OCCLUDERS_ONLY): only rigid geometry may hide forest.
        // Wind, skinning and debris deform away from these bounds, and a body
        // hidden from some camera must not hide what that camera sees.
        if (u32(view.fog_cull.w)&OCCLUDERS_ONLY)!=0u && (inst.flags&(DEFORMING|0xff00u))!=0u {return 0u;}
    }
    // Runtime groups replace this entire branch in the colour view only.
    // Do not spend cluster/raster work on the hidden source. Both sun and
    // perspective lamp shadows retain the same animated source geometry.
    let replacement=cull_voxel_replacement_at(member_slot(id));
    if replacement!=0xffffffffu && (replacement&0x80000000u)!=0u {
        let word=foliage_selection_word(replacement&0x7fffffffu);
        let shadow=view.params.z>0.5 || view.extra.y>1.5;
        if (word&select(1024u,2048u,shadow))!=0u {return 0u;}
    }
    if view.params.z<0.5 && view.extra.y<1.5 && replacement!=0xffffffffu
        && (replacement&0x80000000u)!=0u {
        if foliage_selection_at(replacement&0x7fffffffu).x!=0u {return 0u;}
    }
    // Rasterized voxels cast this group into this sun map instead
    // (selection w 2, see voxel_sun_splats).
    if view.params.z>0.5 && replacement!=0xffffffffu && (replacement&0x80000000u)!=0u
        && foliage_selection_at(replacement&0x7fffffffu).w==2u {return 0u;}
    // Far trees (FAR_TREE tag): rasterized voxels draw the whole owner past
    // the switch distance, and always in the shared sun map once its view
    // marks voxel casting.
    if VOXEL_FAR_M>0.0 && replacement!=0xffffffffu && (replacement&0xc0000000u)==0x40000000u {
        if view.params.z<0.5 && view.extra.y<1.5
            && distance(vec3<f32>(inst.r0.w,inst.r1.w,inst.r2.w),view.cam.xyz)>VOXEL_FAR_M {return 0u;}
        if view.params.z>0.5 && foliage_selection_at(0).w==2u {return 0u;}
    }
    // Per-view hiding (scene::HIDE_IN_VIEW): a player's own body is absent
    // from that player's camera, yet still casts shadows and shows elsewhere.
    // grass.z holds 1 + the camera view index; other passes leave it zero.
    if view.params.z<0.5 && view.extra.y<1.5 && view.grass.z>0.5 {
        let bit=1u<<(u32(view.grass.z)-1u);
        if ((inst.flags>>8u)&bit)!=0u {return 0u;}
    }
    // The orthographic pass is the sun shadow map. Low ground cover only
    // receives shadows; exclude its tiny cards from this expensive pass.
    if (view.params.z > 0.5 || view.extra.y > 1.5) && (inst.flags & 4u) != 0u {
        return 0u;
    }
    // Exactly the same root, camera and opacity decision as raster.vs.
    // Transition grass still renders in its own pass; no geometry is removed
    // from that pass and no threshold, alpha or source plant is changed.
    if EARLY_GRASS_CULL && (inst.flags & 8u) != 0u {
        let root = vec3<f32>(inst.r0.w, inst.r1.w, inst.r2.w);
        if grass_card_opacity(root, view.cam.xyz, view.grass.xy, view.extra.w) < GRASS_OPAQUE_THRESHOLD {
            return 2u;
        }
    }
    return 1u;
}


// A GPU aggregate decision may retain a measured world-error triangle cut.
// Negative means the legacy screen-error policy. Shadows are independent.
fn aggregate_triangle_error(id:u32)->f32 {
    if view.params.z>=0.5 || view.extra.y>=1.5 {return -1.0;}
    let replacement=cull_voxel_replacement_at(member_slot(id));
    if replacement==0xffffffffu || (replacement&0x80000000u)==0u {return -1.0;}
    let selected=foliage_selection_at(replacement&0x7fffffffu);
    if selected.w==0u {return -1.0;}
    return bitcast<f32>(selected.z);
}

fn virtual_owner(inst: Instance) -> bool {
    return inst.cluster_count == 0u && inst.first_cluster != 0u && (inst.flags & 32u) != 0u;
}

var<workgroup> branch_keep: bool;
var<workgroup> cull_owner: u32;
const NO_OWNER: u32 = 0xffffffffu;
var<workgroup> member_plan: u32;

// One instance per workgroup, so everything below is the same in every lane,
// but WGSL's uniformity analysis (enforced by browsers) cannot see that through
// storage reads. Lane 0 decides whatever guards a barrier and shares it with a
// uniform load, which is a barrier as well.
fn cull_instance(id: u32, li: u32) {
    if id >= INSTANCE_COUNT { return; }
    let inst = instances[id];
    if !virtual_owner(inst) { cull_member(id, inst, li); return; }
    // A virtual branch: its own policy (a branch on voxels is skipped
    // whole) and bounds first, then each template composed with its pose.
    if li == 0u {
        branch_keep = false;
        if instance_policy(id, inst) == 1u {
            let radius = inst.sphere.w + cull_wind[id].z;
            branch_keep = in_frustum(inst.sphere.xyz, radius) && !hidden_by_camera_fog(inst.sphere.xyz, radius);
        }
    }
    if !workgroupUniformLoad(&branch_keep) { return; }
    let start = inst.first_cluster >> 12u;
    let count = inst.first_cluster & 0xfffu;
    for (var k = 0u; k < count; k++) {
        cull_member(member_id(id,k), compose_member(inst, instances[start + k]), li);
        workgroupBarrier();
    }
}

const PLAN_SKIP: u32 = 0u;
const PLAN_FLAT: u32 = 1u;
const PLAN_TRAVERSE: u32 = 2u;

fn cull_member(id: u32, inst: Instance, li: u32) {
    // Keep car body geometry consistent in the visible and sun passes. A
    // coarse shadow mesh can cut across its own finer visible panels.
    let base_thr = select(view.params.x, 0.0, (inst.flags & 2u) != 0u);
    if li == 0u {
        member_plan = PLAN_SKIP;
        let policy = instance_policy(id, inst);
        if policy == 2u { atomicAdd(&cull_draw.counter[3], 1u); }
        if policy == 1u {
            let wind_pad = cull_wind[member_slot(id)].z;
            let radius = inst.sphere.w + wind_pad;
            var keep = in_frustum(inst.sphere.xyz, radius) && !hidden_by_camera_fog(inst.sphere.xyz, radius);
            if keep && tree_phase.x>1.5 {
                atomicAdd(&cull_draw.counter[15],1u);
                if tree_occluded(inst.sphere.xyz,radius) {keep=false;atomicAdd(&cull_draw.counter[16],1u);}
            }
            if keep {
                let camera_groups=(spatial_header().w&1u)!=0u && view.params.z<0.5 && view.extra.y<1.5;
                let sun_groups=(spatial_header().w&2u)!=0u && view.params.z>0.5;
                let headlight_groups=(spatial_header().w&4u)!=0u && view.params.z<0.5 && view.extra.y>1.5;
                if camera_groups || sun_groups || headlight_groups {
                    atomicAdd(&cull_draw.counter[26],inst.cluster_count);
                }
                member_plan = PLAN_FLAT;
                // Shared cluster BVH only accelerates camera predicates. Shadow caster
                // thresholds and independently selected offscreen casters keep the flat path.
                if view.params.z<0.5 && view.extra.y<1.5 && aggregate_triangle_error(id)<0.0
                    && cluster_traverse_begin(id,inst,base_thr,wind_pad) {member_plan = PLAN_TRAVERSE;}
            }
        }
    }
    let plan = workgroupUniformLoad(&member_plan);
    if plan == PLAN_SKIP { return; }
    let wind_pad = cull_wind[member_slot(id)].z;
    if plan == PLAN_TRAVERSE {
        cluster_traverse(id,inst,li,base_thr,wind_pad);
        return;
    }
    for (var c = li; c < inst.cluster_count; c += 64u) {
        cull_cluster(id,inst,inst.first_cluster+c,base_thr,wind_pad);
    }
}

// The exact reference predicate is shared by flat and accelerated traversal.
fn cull_cluster(id:u32,inst:Instance,ci:u32,base_thr:f32,wind_pad:f32) {
    let max_visible = u32(view.params.w);
    let cl = clusters[ci];
    // Continuous-alpha surfaces are drawn separately after opaque HDR.
    // Excluding them here also prevents opaque glass sun shadows.
    if (cull_material(cl.material).flags & 256u) != 0u { return; }
    // Foliage seed: cutout, grass and thin foliage cards are not solid.
    if view.params.z<0.5 && view.extra.y<1.5 && (u32(view.fog_cull.w)&OCCLUDERS_ONLY)!=0u
        && (cull_material(cl.material).flags&SEE_THROUGH)!=0u {return;}
    var thr=base_thr;
    // Diagnostic fixed WORLD error for opaque terrain sun casters only.
    // Parent and child use the same threshold, preserving the DAG cut.
    // Camera and perspective headlights never set the reserved uniform.
    if view.params.z>0.5 && view.fog_cull.z>0.0 && (cull_material(cl.material).flags&2u)!=0u {
        thr=min(thr,view.fog_cull.z*view.cam.w);
    }
    if view.params.z>0.5 && view.fog_cull.w>0.5 && (cull_material(cl.material).flags&6u)!=0u {
        thr=0.0;
    }
    let s = inst.scale;
    // Shear can stretch simplification error as well as move bounds.
    // ||I + shear|| <= 1 + |shear|; use the identical factor on both
    // sides of every parent/child cut so the DAG stays complete.
    let error_scale = s * (1.0 + length(cull_wind[member_slot(id)].xy));
    let pc = to_world(inst, cl.parent.xyz);
    let aggregate_error=aggregate_triangle_error(id);
    var parent_px = select(projected(cl.parent_error * error_scale, pc, cl.parent.w * s + wind_pad), 1e30, cl.parent_error > 1e30);
    if aggregate_error>=0.0 {
        thr=aggregate_error;
        parent_px=select(cl.parent_error*error_scale,1e30,cl.parent_error>1e30);
    }
    if parent_px <= thr {
        return; // a coarser cluster covers this patch
    }
    // Every cluster, the DAG's roots included, must itself be fine
    // enough; where it is not, its finer clusters are drawn instead.
    let oc = to_world(inst, cl.own.xyz);
    var own_px=projected(cl.own_error * error_scale, oc, cl.own.w * s + wind_pad);
    if aggregate_error>=0.0 {own_px=cl.own_error*error_scale;}
    if own_px > thr {
        return;
    }
    let cc = to_world(inst, cl.cull.xyz);
    let radius = cl.cull.w * s + wind_pad;
    if !in_frustum(cc, radius) || hidden_by_camera_fog(cc, radius) {
        return;
    }
    // Current receiver demand may omit only ordinary Sun casters whose full
    // light-space footprint misses every requested tile. No camera visibility,
    // receiver Z range, material or shadow LOD substitutes for this test.
    if !receiver_shadow_keep(cc,radius) {return;}
    if tree_phase.x>0.5 && tree_phase.w>0.5 {
        let key=tree_history_keys(id,ci);
        let a=(tree_history[key.x>>7u][(key.x>>5u)&3u]&(1u<<(key.x&31u)))!=0u;
        let b=(tree_history[key.y>>7u][(key.y>>5u)&3u]&(1u<<(key.y&31u)))!=0u;
        // Empty coverage proofs cannot save work. Render the complete current cut
        // directly until the periodic GPU-only probe; this never hides geometry.
        // Every 4th frame, so geometry that just went out of sight (turning
        // towards a building) stops drawing within 4 frames, not 32.
        let policy=tree_history[2048];
        let direct=policy.y==0u && (policy.x&3u)!=0u;
        // Every category: what was visible last frame draws first and seeds
        // the Hi-Z; everything else (buildings, rocks, terrain patches, cars,
        // trees) is tested against it in phase two.
        let first=direct || (a && b);
        if tree_phase.x<1.5 && !first {
            let pending=atomicAdd(&cull_draw.counter[19],1u);
            if pending<u32(tree_phase.y) {
                let at=max_visible+pending*3u;
                visible[at]=vec2<u32>(id,ci);
                visible[at+1u]=bitcast<vec2<u32>>(cc.xy);
                visible[at+2u]=bitcast<vec2<u32>>(vec2<f32>(cc.z,radius));
            }
            return;
        }
        if tree_phase.x>1.5 && first {return;}
    }
    if tree_phase.x>1.5 {
        atomicAdd(&cull_draw.counter[17],1u);
        if tree_occluded(cc,radius) {atomicAdd(&cull_draw.counter[18],1u);return;}
    }
    let slot = atomicAdd(&cull_draw.counter[0], 1u);
    if slot < max_visible {
        visible[slot] = vec2<u32>(id, ci);
        atomicAdd(&cull_draw.counter[1], cl.tri_count);
        if CLUSTER_BUCKETS { count_cluster(inst.flags, cl); }
    }
}

@compute @workgroup_size(64)
fn cull(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    cull_instance(wg.x + wg.y * 65535u, li);
}

@compute @workgroup_size(64)
fn cull_sun_static(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let slot=wg.x+wg.y*65535u;
    if slot>=sun_owner(0) {return;}
    cull_instance(sun_owner(sun_owner(2)+slot),li);
}

@compute @workgroup_size(64)
fn cull_sun_dynamic(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let slot=wg.x+wg.y*65535u;
    if slot>=sun_owner(1) {return;}
    cull_instance(sun_owner(sun_owner(3)+slot),li);
}

// draw_map is scratch until cluster selection has finished; the later bucket
// scatter overwrites it. Counter 11 is likewise unused until that scatter.
// No extra storage binding or large owner-list allocation is needed.
@compute @workgroup_size(64)
fn instance_broadphase(@builtin(global_invocation_id) gid: vec3<u32>) {
    let id = gid.x + gid.y * 65535u * 64u;
    if id >= INSTANCE_COUNT { return; }
    let inst = instances[id];
    let policy = instance_policy(id, inst);
    if policy == 2u { atomicAdd(&cull_draw.counter[3], 1u); }
    if policy != 1u { return; }
    let radius = inst.sphere.w + cull_wind[member_slot(id)].z;
    if !in_frustum(inst.sphere.xyz, radius) || hidden_by_camera_fog(inst.sphere.xyz, radius) { return; }
    let slot = atomicAdd(&cull_draw.counter[11], 1u);
    if slot < arrayLength(&cull_draw.draw_map) { cull_draw.draw_map[slot] = id; }
}

@compute @workgroup_size(1)
fn instance_dispatch() {
    let count = atomicLoad(&cull_draw.counter[11]);
    let overflow = count > arrayLength(&cull_draw.draw_map);
    let dispatch_count = select(count, INSTANCE_COUNT, overflow);
    cull_draw.args[0] = min(dispatch_count, 65535u);
    cull_draw.args[1] = (dispatch_count + 65534u) / 65535u;
    cull_draw.args[2] = 1u;
    // An overfull scratch list falls back to the exact original full scan.
    // That scan recounts early grass rejects, so do not count them twice.
    if overflow { atomicStore(&cull_draw.counter[3], 0u); }
}

@compute @workgroup_size(64)
fn cull_compacted(@builtin(workgroup_id) wg: vec3<u32>, @builtin(local_invocation_index) li: u32) {
    let slot = wg.x + wg.y * 65535u;
    if li == 0u {
        let count = atomicLoad(&cull_draw.counter[11]);
        let overflow = count > arrayLength(&cull_draw.draw_map);
        cull_owner = NO_OWNER;
        if slot < select(count, INSTANCE_COUNT, overflow) {
            cull_owner = slot;
            if !overflow { cull_owner = cull_draw.draw_map[slot]; }
        }
    }
    cull_instance(workgroupUniformLoad(&cull_owner), li);
}

fn spatial_group_visible(sphere:vec4<f32>)->bool {
    if sphere.w<0.0 {return false;} // refit proved every member empty
    // Invalid or extreme member bounds retain the group, never certify a
    // rejection. Individual policy below remains the established reference.
    if !all(abs(sphere)<vec4<f32>(1e19)) {return true;}
    if !in_frustum(sphere.xyz,sphere.w) || hidden_by_camera_fog(sphere.xyz,sphere.w) {return false;}
    return tree_phase.x<=1.5 || !tree_occluded(sphere.xyz,sphere.w);
}
// Actual light-space bounds only. A tree outside every player's camera can
// still block a light: camera depth/fog/occlusion must never reject it here.
// The six planes also describe finite perspective headlight frusta; this
// predicate does not assume an orthographic projection.
fn sun_group_visible(sphere:vec4<f32>)->bool {
    if sphere.w<0.0 {return false;}
    if !all(abs(sphere)<vec4<f32>(1e19)) {return true;}
    return in_frustum(sphere.xyz,sphere.w);
}
var<workgroup> spatial_parent_keep:bool;
var<workgroup> spatial_leaf_keep:bool;

fn spatial_select_owner(id:u32,sun:bool) {
    let inst=instances[id];atomicAdd(&cull_draw.counter[24],1u);
    atomicAdd(&cull_draw.counter[30],1u);
    let policy=instance_policy(id,inst);
    if policy==2u {atomicAdd(&cull_draw.counter[28],1u);}
    if policy!=1u {return;}
    let radius=inst.sphere.w+cull_wind[member_slot(id)].z;
    if !in_frustum(inst.sphere.xyz,radius) {return;}
    if !sun && hidden_by_camera_fog(inst.sphere.xyz,radius) {return;}
    let slot=atomicAdd(&cull_draw.counter[11],1u);atomicAdd(&cull_draw.counter[25],1u);
    // The decision tail is not compacted-owner capacity.
    if slot<spatial_header().z {spatial_owners[slot]=id;}
}

// One workgroup per512-owner spatial parent, all64 lanes per retained leaf.
// Rejected parents read no leaf bounds/owners; rejected leaves read no owners.
// Refitted spheres include current source bounds and exact wind padding.
// Keep the leaf loop uniform: every lane reaches both barriers even when its
// owner is empty/outside or the final leaf has fewer than64 members. The old
// divergent eight-lane traversal lost most retained owners on the tested GPU.
fn select_spatial_parent(parent_id:u32,li:u32,sun:bool) {
    if parent_id>=spatial_header().y {return;}
    let parent=spatial_group(spatial_header().x+parent_id);
    if li==0u {
        atomicAdd(&cull_draw.counter[20],1u);
        if sun {spatial_parent_keep=sun_group_visible(parent.sphere);}
        else {spatial_parent_keep=spatial_group_visible(parent.sphere);}
        if !spatial_parent_keep {atomicAdd(&cull_draw.counter[21],1u);}
    }
    if !workgroupUniformLoad(&spatial_parent_keep) {return;}
    for(var leaf_id=0u;leaf_id<parent.span.y;leaf_id++) {
        let leaf=spatial_group(parent.span.x+leaf_id);
        if li==0u {
            atomicAdd(&cull_draw.counter[22],1u);
            if sun {spatial_leaf_keep=sun_group_visible(leaf.sphere);}
            else {spatial_leaf_keep=spatial_group_visible(leaf.sphere);}
            if spatial_leaf_keep {atomicAdd(&cull_draw.counter[29],leaf.span.y);}
            else {atomicAdd(&cull_draw.counter[23],1u);}
        }
        workgroupBarrier();
        if spatial_leaf_keep && li<leaf.span.y {
            spatial_select_owner(spatial_member(leaf.span.x+li),sun);
        }
        workgroupBarrier();
    }
}
@compute @workgroup_size(64)
fn spatial_select(@builtin(workgroup_id) wg:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    select_spatial_parent(wg.x+wg.y*65535u,li,false);
}
@compute @workgroup_size(64)
fn sun_spatial_select(@builtin(workgroup_id) wg:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    select_spatial_parent(wg.x+wg.y*65535u,li,true);
}
@compute @workgroup_size(64)
fn headlight_spatial_select(@builtin(workgroup_id) wg:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    // Same current-pose bounds, independently tested against this lamp's
    // perspective planes. No camera retained-owner list, fog or Hi-Z.
    select_spatial_parent(wg.x+wg.y*65535u,li,true);
}
@compute @workgroup_size(1)
fn spatial_dispatch() {
    let count=atomicLoad(&cull_draw.counter[11]);let overflow=count>spatial_header().z;
    let mismatch=atomicLoad(&cull_draw.counter[29])!=atomicLoad(&cull_draw.counter[30]);
    let fallback=overflow || mismatch;
    let n=select(count,INSTANCE_COUNT,fallback);
    cull_draw.args[0]=min(n,65535u);cull_draw.args[1]=(n+65534u)/65535u;cull_draw.args[2]=1u;
    if fallback {
        // The consumer already recognizes an over-capacity count as the exact
        // original full-owner scan. Never trust a partially visited list.
        atomicStore(&cull_draw.counter[11],spatial_header().z+1u);
        atomicAdd(&cull_draw.counter[27],1u);
        if mismatch {atomicAdd(&cull_draw.counter[31],1u);}
    }else{atomicAdd(&cull_draw.counter[3],atomicLoad(&cull_draw.counter[28]));}
    atomicStore(&cull_draw.counter[28],0u);
    atomicStore(&cull_draw.counter[29],0u);atomicStore(&cull_draw.counter[30],0u);
}
// The owner this workgroup's slot holds, or NO_OWNER past the end of the list.
fn spatial_owner(slot:u32,li:u32)->u32 {
    if li==0u {
        let count=atomicLoad(&cull_draw.counter[11]);let overflow=count>spatial_header().z;
        cull_owner=NO_OWNER;
        if slot<select(count,INSTANCE_COUNT,overflow) {
            cull_owner=slot;
            if !overflow {cull_owner=spatial_owners[slot];}
        }
    }
    return workgroupUniformLoad(&cull_owner);
}
@compute @workgroup_size(64)
fn cull_spatial(@builtin(workgroup_id) wg:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    cull_instance(spatial_owner(wg.x+wg.y*65535u,li),li);
}

// Keep visible[] and its IDs unchanged. Only draw order and padded vertex
// count change. first_vertex encodes the bucket, avoiding any dependency on
// the optional INDIRECT_FIRST_INSTANCE device feature.
@compute @workgroup_size(1)
fn finalize() {
    // The optional broadphase temporarily uses the final bucket cursor.
    // Its list has now been consumed, so scatter owns all four cursors again.
    for (var cursor = 8u; cursor < 12u; cursor++) { atomicStore(&cull_draw.counter[cursor], 0u); atomicStore(&cull_draw.counter[cursor+36u], 0u); }
    let n = min(atomicLoad(&cull_draw.counter[0]), u32(view.params.w));
    let first=atomicLoad(&cull_draw.counter[12]);
    if tree_phase.x>1.5 {atomicStore(&cull_draw.counter[14],n-first);}
    else if tree_phase.x>0.5 {atomicStore(&cull_draw.counter[13],n);}
    if !CLUSTER_BUCKETS {
        cull_draw.args[0] = 384u;
        cull_draw.args[1] = n;
        cull_draw.args[2] = 0u;
        cull_draw.args[3] = 0u;
        atomicStore(&cull_draw.counter[2], n * 384u);
        return;
    }
    var start = 0u;
    var submitted = 0u;
    for (var bucket = 0u; bucket < 4u; bucket++) {
        let count = atomicLoad(&cull_draw.counter[4u + bucket]);
        let masked = select(0u, min(atomicLoad(&cull_draw.counter[40u + bucket]), count), RASTER_SPLIT);
        let vertices = bucket_vertices(bucket);
        cull_draw.args[bucket * 4u] = vertices;
        cull_draw.args[bucket * 4u + 1u] = count - masked;
        cull_draw.args[bucket * 4u + 2u] = bucket * 384u;
        cull_draw.args[bucket * 4u + 3u] = 0u;
        cull_draw.args[16u + bucket] = start;
        // Masked records: first_vertex (4+bucket)*384 selects their start.
        cull_draw.args[20u + bucket * 4u] = vertices;
        cull_draw.args[21u + bucket * 4u] = masked;
        cull_draw.args[22u + bucket * 4u] = (4u + bucket) * 384u;
        cull_draw.args[23u + bucket * 4u] = 0u;
        cull_draw.args[36u + bucket] = start + count - masked;
        start += count;
        submitted += count * vertices;
    }
    // Phase2 adds to phase1 work; visibility prefix and triangle totals live
    // through both passes. Reference/phase1 start from zero.
    atomicAdd(&cull_draw.counter[2], submitted);
}

// Encoded only after phase1 raster and its pyramid. This is GPU-ordered,
// unlike a second queue.write_buffer before the command buffer is submitted.
@compute @workgroup_size(1)
fn prepare_tree_phase() {
    atomicStore(&cull_draw.counter[12],min(atomicLoad(&cull_draw.counter[0]),u32(view.params.w)));
    for(var i=4u;i<12u;i++){atomicStore(&cull_draw.counter[i],0u);atomicStore(&cull_draw.counter[i+36u],0u);}
    if tree_phase.w>0.5 {
        let pending=atomicLoad(&cull_draw.counter[19]);
        let groups=select((pending+63u)/64u,INSTANCE_COUNT,pending>u32(tree_phase.y));
        cull_draw.args[0]=min(groups,65535u);cull_draw.args[1]=(groups+65534u)/65535u;cull_draw.args[2]=1u;
    }
}
// Phase one already did current LOD/frustum selection. Recover only deferred
// clusters, not a second full-world traversal. Queue overflow uses the complete
// complementary scan, preserving the first-pass visibility prefix.
@compute @workgroup_size(64)
fn recover_tree_clusters(@builtin(workgroup_id) group:vec3<u32>,@builtin(local_invocation_index) lane:u32) {
    if lane==0u {tree_pending=atomicLoad(&cull_draw.counter[19]);}
    workgroupBarrier();
    let work=group.x+group.y*65535u;let pending=workgroupUniformLoad(&tree_pending);
    if pending>u32(tree_phase.y) {cull_instance(work,lane);return;}
    if lane<11u {atomicStore(&tree_group_counts[lane],0u);}
    workgroupBarrier();
    let index=work*64u+lane;var record=vec2<u32>(0u);var keep=false;
    if index<pending {
        let at=u32(view.params.w)+index*3u;record=visible[at];
        let xy=bitcast<vec2<f32>>(visible[at+1u]);let zr=bitcast<vec2<f32>>(visible[at+2u]);
        keep=!tree_occluded(vec3<f32>(xy,zr.x),zr.y);
        if !keep {atomicAdd(&tree_group_counts[6],1u);}
    }
    var local=0u;if keep {local=atomicAdd(&tree_group_counts[0],1u);}
    workgroupBarrier();
    if lane==0u {
        tree_output_base=atomicAdd(&cull_draw.counter[0],atomicLoad(&tree_group_counts[0]));
        atomicAdd(&cull_draw.counter[17],min(64u,pending-work*64u));
        atomicAdd(&cull_draw.counter[18],atomicLoad(&tree_group_counts[6]));
    }
    workgroupBarrier();
    if keep && tree_output_base+local<u32(view.params.w) {
        visible[tree_output_base+local]=record;let triangles=clusters[record.y].tri_count;
        atomicAdd(&tree_group_counts[1],triangles);
        if CLUSTER_BUCKETS {
            let bucket=cluster_bucket(triangles);
            atomicAdd(&tree_group_counts[2u+bucket],1u);
            if RASTER_SPLIT && masked_cluster(instance_at(record.x).flags,clusters[record.y].material) {atomicAdd(&tree_group_counts[7u+bucket],1u);}
        }
    }
    workgroupBarrier();
    if lane==0u {atomicAdd(&cull_draw.counter[1],atomicLoad(&tree_group_counts[1]));}
    if CLUSTER_BUCKETS && lane<4u {
        atomicAdd(&cull_draw.counter[4u+lane],atomicLoad(&tree_group_counts[2u+lane]));
        atomicAdd(&cull_draw.counter[40u+lane],atomicLoad(&tree_group_counts[7u+lane]));
    }
}
var<workgroup> tree_pending:u32;
var<workgroup> tree_output_base:u32;
var<workgroup> tree_group_counts:array<atomic<u32>,11>;

@compute @workgroup_size(64)
fn scatter(@builtin(global_invocation_id) gid: vec3<u32>, @builtin(num_workgroups) groups: vec3<u32>) {
    let n = min(atomicLoad(&cull_draw.counter[0]), u32(view.params.w));
    for (var slot = atomicLoad(&cull_draw.counter[12])+gid.x; slot < n; slot += groups.x * 64u) {
        let vis = visible[slot];
        let cl = clusters[vis.y];
        let bucket = cluster_bucket(cl.tri_count);
        if RASTER_SPLIT && masked_cluster(instance_at(vis.x).flags, cl.material) {
            // From the bucket's end: a miscount can only change which
            // pipeline draws a cluster, never leave a draw_map entry stale.
            let offset = atomicAdd(&cull_draw.counter[44u + bucket], 1u);
            cull_draw.draw_map[cull_draw.args[16u + bucket] + atomicLoad(&cull_draw.counter[4u + bucket]) - 1u - offset] = slot;
        } else {
            let offset = atomicAdd(&cull_draw.counter[8u + bucket], 1u);
            cull_draw.draw_map[cull_draw.args[16u + bucket] + offset] = slot;
        }
    }
}

// Flat scenes without vegetation/replacement experiments can assign one lane
// per owner, avoiding a full workgroup for every empty slot and one-cluster block.
@compute @workgroup_size(64)
fn cull_simple(@builtin(global_invocation_id) gid:vec3<u32>) {
    let id=gid.x+gid.y*65535u*64u;
    if id>=INSTANCE_COUNT {return;}
    if view.extra.y>1.5 && view.grass.w>0.0 {
        let first=u32(view.grass.z); let count=u32(view.grass.w);
        if id>=first && id-first<count {return;}
    }
    let inst=instances[id];
    if inst.cluster_count==0u || inst.sphere.w<0.0 {return;}
    let shadow=view.params.z>0.5 || view.extra.y>1.5;
    if shadow && (inst.flags&4u)!=0u {return;}
    if !shadow && view.grass.z>0.5 && ((inst.flags>>8u)&(1u<<(u32(view.grass.z)-1u)))!=0u {return;}
    if !in_frustum(inst.sphere.xyz,inst.sphere.w) || hidden_by_camera_fog(inst.sphere.xyz,inst.sphere.w) {return;}
    let threshold=select(view.params.x,0.0,(inst.flags&2u)!=0u);
    for(var offset=0u;offset<inst.cluster_count;offset++) {
        let ci=inst.first_cluster+offset;let cl=clusters[ci];
        if (cull_material(cl.material).flags&256u)!=0u {continue;}
        let parent=to_world(inst,cl.parent.xyz);
        if cl.parent_error<=1e30 && projected(cl.parent_error*inst.scale,parent,cl.parent.w*inst.scale)<=threshold {continue;}
        let own=to_world(inst,cl.own.xyz);
        if projected(cl.own_error*inst.scale,own,cl.own.w*inst.scale)>threshold {continue;}
        if !in_frustum(to_world(inst,cl.cull.xyz),cl.cull.w*inst.scale) {continue;}
        let slot=atomicAdd(&cull_draw.counter[0],1u);
        if slot<u32(view.params.w) {
            visible[slot]=vec2<u32>(id,ci);atomicAdd(&cull_draw.counter[1],cl.tri_count);
            if CLUSTER_BUCKETS {count_cluster(inst.flags,cl);}
        }
    }
}

// Only cached sun entry points reference group 2. Normal camera, near sun and
// compatibility paths keep their existing layouts and visibility semantics.
@group(2) @binding(0) var<uniform> shadow_damage_words:array<vec4<u32>,32>;
var<workgroup> damaged_owner:bool;
fn cull_damaged_instance(id:u32,li:u32) {
    if id>=INSTANCE_COUNT {return;}
    if li==0u {
        let inst=instances[id];
        let radius=max(inst.sphere.w,0.)+max(cull_wind[id].z,0.);
        damaged_owner=shadow_damage_bounds(view.view_proj,view.size.x,
            inst.sphere.xyz-vec3<f32>(radius),inst.sphere.xyz+vec3<f32>(radius));
    }
    if !workgroupUniformLoad(&damaged_owner) {return;}
    cull_instance(id,li);
}
@compute @workgroup_size(64)
fn cull_cached_sun(@builtin(workgroup_id) wg:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    cull_damaged_instance(wg.x+wg.y*65535u,li);
}
@compute @workgroup_size(64)
fn cull_spatial_cached_sun(@builtin(workgroup_id) wg:vec3<u32>,@builtin(local_invocation_index) li:u32) {
    cull_damaged_instance(spatial_owner(wg.x+wg.y*65535u,li),li);
}
