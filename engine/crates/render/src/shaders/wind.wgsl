// One evaluation per instance, reused across every camera, cluster and pass.
struct WindUniform { field: vec4<f32>, amplitudes: vec4<f32> };
struct WindInstance {
    r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32>, sphere: vec4<f32>,
    first_cluster: u32, cluster_count: u32, scale: f32, flags: u32,
};
@group(0) @binding(0) var<uniform> wind: WindUniform;
@group(0) @binding(1) var<storage,read> wind_instances: array<WindInstance>;
@group(0) @binding(2) var<storage,read> wind_voxel_replacement: array<u32>;
@group(0) @binding(3) var<storage,read_write> output: array<vec4<f32>>;
// Trees farther than `params.y` from every camera stand still: their shadow
// casters can then be cached (static_sun.rs). params = (fade start, frozen
// beyond, camera count, enabled); cameras[k].xz is a camera's position.
struct WindFreeze { params: vec4<f32>, cameras: array<vec4<f32>, 4> };
@group(0) @binding(4) var<uniform> freeze: WindFreeze;
@group(0) @binding(5) var<storage,read> tree_slots: array<u32>;

// A batch header stores its min/max root height and its offset into this palette.
// Following the physical instance records are the planted world roots; shared
// camera and shadow batches use the same indices. Ordinary scenes bind one zero.
@group(0) @binding(6) var<storage,read> batch_roots: array<vec4<f32>>;

fn field_shear(root: vec2<f32>, grass: bool) -> vec2<f32> {
    var still = 1.0;
    if !grass && freeze.params.w != 0.0 {
        var nearest = 1e30;
        for (var k = 0u; k < u32(freeze.params.z); k++) {
            nearest = min(nearest, distance(root, freeze.cameras[k].xz));
        }
        still = 1.0 - smoothstep(freeze.params.x, freeze.params.y, nearest);
        if nearest >= freeze.params.y { return vec2<f32>(0.0); }
    }
    let amplitude = select(wind.amplitudes.x,wind.amplitudes.y,grass);
    return plant_gust(root, wind.field.xy, wind.field.w) * (amplitude * wind.field.z * still);
}

@compute @workgroup_size(64)
fn update_wind(@builtin(global_invocation_id) id: vec3<u32>) {
    let owner = id.x;
    if owner >= arrayLength(&output) { return; }
    output[owner] = vec4<f32>(0.0);
    if wind.field.z == 0.0 { return; }
    if owner >= arrayLength(&wind_instances) {
        let root = batch_roots[owner];
        if root.w < 0.0 {
            // Integer length is exact; raster and motion use it to bound tags.
            output[owner].w = -root.w;
        } else {
            output[owner] = vec4<f32>(field_shear(root.xz, false), 0.0, root.y);
        }
        return;
    }
    let inst = wind_instances[owner];
    if inst.cluster_count == 0u || (inst.flags & 32u) != 0u { return; }
    if owner < arrayLength(&batch_roots) {
        let batch = batch_roots[owner];
        if batch.w > 0.0 {
            // Cull with the worst possible member shear. Freeze only once the
            // entire batch lies beyond every camera, including its outer roots.
            if freeze.params.w != 0.0 {
                let root_circle = batch_roots[u32(batch.w) - 1u];
                var nearest = 1e30;
                for (var k = 0u; k < u32(freeze.params.z); k++) {
                    nearest = min(nearest, distance(root_circle.xy, freeze.cameras[k].xz));
                }
                if nearest >= freeze.params.y + root_circle.z { return; }
            }
            let amplitude = wind.amplitudes.x * wind.field.z;
            let height = max(abs(inst.sphere.y-batch.x),abs(inst.sphere.y-batch.y)) + inst.sphere.w;
            output[owner] = vec4<f32>(amplitude, 0.0, height * amplitude, batch.w);
            return;
        }
    }
    if (inst.flags & 25u) == 0u {
        let word = owner >> 5u;
        if word >= arrayLength(&tree_slots) { return; }
        if (tree_slots[word] & (1u << (owner & 31u))) == 0u { return; }
    }
    if wind_voxel_replacement[owner] != 0u
        && (wind_voxel_replacement[owner] & 0xc0000000u) != 0x40000000u { return; }
    let shear = field_shear(vec2<f32>(inst.r0.w, inst.r2.w), (inst.flags & 24u) != 0u);
    // Triangle vertices lie in the original instance sphere. Height above the
    // anchor is bounded by |centre.y-root.y|+radius, even for unusual meshes.
    let displacement = (abs(inst.sphere.y-inst.r1.w)+inst.sphere.w)*length(shear);
    output[owner] = vec4<f32>(shear,displacement,0.0);
}
