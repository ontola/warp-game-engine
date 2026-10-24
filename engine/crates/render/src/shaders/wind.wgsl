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

@compute @workgroup_size(64)
fn update_wind(@builtin(global_invocation_id) id: vec3<u32>) {
    let owner = id.x;
    if owner >= arrayLength(&wind_instances) || owner >= arrayLength(&output) { return; }
    let inst = wind_instances[owner];
    output[owner] = vec4<f32>(0.0);
    // FOLIAGE=1, LIVE_GRASS=8, WIND_GRASS=16. A volume crown and its source
    // trunk/shadows remain at rest together until volume deformation is ready.
    if inst.cluster_count == 0u || (inst.flags & 25u) == 0u || (inst.flags & 32u) != 0u
        || wind_voxel_replacement[owner] != 0u || wind.field.z == 0.0 { return; }
    let direction = wind.field.xy;
    let across_dir = vec2<f32>(-direction.y, direction.x);
    let root = vec2<f32>(inst.r0.w, inst.r2.w);
    let along = dot(root,direction);
    let across = dot(root,across_dir);
    let phase = wind.field.w;
    let a = along*0.12 - phase*9.0;
    let b = along*0.057 + across*0.033 - phase*17.0;
    let c = along*0.21 + across*0.37 - phase*41.0;
    let wave = 0.52 + 0.26*sin(a) + 0.09*sin(b);
    let grass = (inst.flags & 24u) != 0u;
    let amplitude = select(wind.amplitudes.x,wind.amplitudes.y,grass);
    let shear = (direction*wave + across_dir*(0.13*sin(c)))*(amplitude*wind.field.z);
    // Triangle vertices lie in the original instance sphere. Height above the
    // anchor is bounded by |centre.y-root.y|+radius, even for unusual meshes.
    let displacement = (abs(inst.sphere.y-inst.r1.w)+inst.sphere.w)*length(shear);
    output[owner] = vec4<f32>(shear,displacement,0.0);
}
