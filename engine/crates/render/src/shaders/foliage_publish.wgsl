// One bounded world delta; unchanged roots and their GPU histories are untouched.
struct Source {rows:array<vec4<f32>,3>,sphere:vec4<f32>,first:u32,count:u32,scale:f32,flags:u32};
struct Volume {inverse:array<vec4<f32>,3>,low:vec4<f32>,high:vec4<f32>,identity:vec4<u32>};
struct Root {rows:array<vec4<f32>,3>,scale:vec4<f32>,info:vec4<u32>};
struct Change {slot:vec4<u32>,root:Root,proxy:Volume,source:Source};
@group(0) @binding(0) var<storage,read> changes:array<Change>;
@group(0) @binding(1) var<storage,read_write> roots:array<Root>;
@group(0) @binding(2) var<storage,read_write> proxies:array<Volume>;
@group(0) @binding(3) var<storage,read_write> sources:array<Source>;
@compute @workgroup_size(64)
fn publish(@builtin(global_invocation_id) id:vec3<u32>) {
 if id.x>=arrayLength(&changes) {return;}
 let change=changes[id.x];
 roots[change.slot.x]=change.root;
 proxies[change.slot.x]=change.proxy;
 sources[change.proxy.identity.x]=change.source;
}
