// One decision per registered coverage group, never per pixel or source cluster.
struct ReplacementView {vp:mat4x4<f32>,output:vec4<f32>,info:vec4<u32>}
@group(0) @binding(0) var<uniform> selection_view:ReplacementView;
@group(0) @binding(1) var<storage,read> packed:array<u32>;
@group(0) @binding(2) var<storage,read_write> selection:array<u32>;
@group(0) @binding(3) var<storage,read> readiness:array<vec4<u32>>;

fn descriptor(group:u32)->u32 {return packed[selection_view.info.x+5u]+group*16u;}
fn pixels_per_metre(center:vec3<f32>,radius:f32)->f32 {
    let rows=transpose(selection_view.vp);let p=vec4<f32>(center,1.0);
    let w=rows[3];let z=rows[2];let wlen=length(w.xyz);
    let wmin=dot(w,p)-radius*wlen;
    let zmin=dot(z,p)-radius*length(z.xyz);
    let wzmin=dot(w-z,p)-radius*length((w-z).xyz);
    // Near-plane/camera overlap or malformed projection always keeps source.
    if !(wmin>1e-6 && zmin>0.0 && wzmin>0.0) {return 1e30;}
    let mx=length(rows[0].xyz);let my=length(rows[1].xyz);
    let nx=abs(dot(rows[0],p))+radius*mx;let ny=abs(dot(rows[1],p))+radius*my;
    let scale=length(vec2<f32>(mx/wmin+nx*wlen/(wmin*wmin),my/wmin+ny*wlen/(wmin*wmin))*selection_view.output.xy*0.5);
    return select(1e30,scale,scale>=0.0 && scale<1e29);
}
@compute @workgroup_size(64)
fn select_groups(@builtin(global_invocation_id) id:vec3<u32>) {
    let group=id.x;if group>=selection_view.info.y {return;}
    let output=selection_view.info.x+8u+group;selection[output]=0u;
    if selection_view.info.w==0u || group>=arrayLength(&readiness) {return;}
    let base=descriptor(group);let ready=readiness[group];
    if ready.x!=packed[base+2u] || ready.y!=1u || ready.z!=1u || (ready.w!=0u && ready.w!=selection_view.info.z) {return;}
    let bound=vec4<f32>(bitcast<f32>(packed[base+4u]),bitcast<f32>(packed[base+5u]),bitcast<f32>(packed[base+6u]),bitcast<f32>(packed[base+7u]));
    let error=bitcast<f32>(packed[base+8u]);
    if !all(abs(bound)<vec4<f32>(1e18)) || !(bound.w>=0.0) || !(error>0.0 && error<1e18) {return;}
    let pixels=error*pixels_per_metre(bound.xyz,bound.w);
    if pixels<=selection_view.output.z {selection[output]=1u;}
}
@compute @workgroup_size(1)
fn publish_selection() {
    let base=selection_view.info.x;
    selection[base]=0x53525031u;selection[base+1u]=selection_view.info.z;
    selection[base+2u]=selection_view.info.z;selection[base+3u]=selection_view.info.y;
    selection[base+4u]=selection_view.info.w;selection[base+5u]=selection_view.info.x;
    selection[base+6u]=0u;selection[base+7u]=0u;
}
