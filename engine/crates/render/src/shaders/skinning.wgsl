struct SkinSource {
    position: vec4<f32>, normal: vec4<f32>,
    joints0: vec4<u32>, joints1: vec4<u32>,
    weights0: vec4<f32>, weights1: vec4<f32>,
    address: vec4<u32>,
};
struct SkinMatrix { r0: vec4<f32>, r1: vec4<f32>, r2: vec4<f32> };
// Scene Vertex is exactly eight packed scalars (32 bytes), not vec3-aligned.
struct SceneVertex { px:f32,py:f32,pz:f32,nx:f32,ny:f32,nz:f32,u:f32,v:f32 };
@group(0) @binding(0) var<storage,read> sources: array<SkinSource>;
@group(0) @binding(1) var<storage,read> palette: array<SkinMatrix>;
@group(0) @binding(2) var<storage,read_write> vertices: array<SceneVertex>;

@compute @workgroup_size(64)
fn skin_vertices(@builtin(global_invocation_id) id:vec3<u32>) {
    if id.x>=arrayLength(&sources) { return; }
    let source=sources[id.x];
    var r0=vec4<f32>(0.0);var r1=r0;var r2=r0;
    for(var i=0u;i<8u;i++) {
        var joint=source.joints0[i%4u];var weight=source.weights0[i%4u];
        if i>=4u {joint=source.joints1[i%4u];weight=source.weights1[i%4u];}
        let bone=palette[source.address.y+joint];
        r0+=bone.r0*weight;r1+=bone.r1*weight;r2+=bone.r2*weight;
    }
    let p=vec3<f32>(dot(r0,source.position),dot(r1,source.position),dot(r2,source.position));
    // Cofactors give the inverse transpose of the blended skin transform.
    // This also supports nonuniform import scale. Degenerate poses retain a
    // finite normal instead of introducing NaNs into the shared scene buffer.
    let c0=cross(r1.xyz,r2.xyz);let c1=cross(r2.xyz,r0.xyz);let c2=cross(r0.xyz,r1.xyz);
    let determinant=dot(r0.xyz,c0);
    var normal=vec3<f32>(dot(c0,source.normal.xyz),dot(c1,source.normal.xyz),dot(c2,source.normal.xyz));
    if determinant<0.0 {normal=-normal;}
    if dot(normal,normal)<1e-16 {normal=source.normal.xyz;}
    normal=normalize(normal);
    let dst=source.address.x;
    vertices[dst].px=p.x;vertices[dst].py=p.y;vertices[dst].pz=p.z;
    vertices[dst].nx=normal.x;vertices[dst].ny=normal.y;vertices[dst].nz=normal.z;
}
