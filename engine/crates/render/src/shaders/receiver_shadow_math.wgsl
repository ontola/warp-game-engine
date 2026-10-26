// Pure orthographic projection used by both receiver requests and caster tests.
// kind: 0 outside map, 1 finite inclusive rectangle, 2 uncertain (keep all).
struct DemandRect { first:vec2<u32>, last:vec2<u32>, kind:u32 }
fn demand_project(vp:mat4x4<f32>,center:vec3<f32>,extent:vec3<f32>,size:u32,guard:f32)->DemandRect {
    var result:DemandRect; result.kind=2u;
    if size==0u || !all(abs(center)<vec3<f32>(1e18)) ||
        !all(extent>=vec3<f32>(0.0)) || !all(extent<vec3<f32>(1e18)) {return result;}
    let rows=transpose(vp);
    if any(abs(rows[3]-vec4<f32>(0.0,0.0,0.0,1.0))>vec4<f32>(1e-6)) {return result;}
    let p=vp*vec4<f32>(center,1.0);
    let radius=vec2<f32>(dot(abs(rows[0].xyz),extent),dot(abs(rows[1].xyz),extent));
    // Absolute-product bound for matmul/extent rounding, including large world
    // origins. It expands requests rather than changing any shadow depth bias.
    let magnitude=vec2<f32>(dot(abs(rows[0]),vec4<f32>(abs(center),1.0)),
        dot(abs(rows[1]),vec4<f32>(abs(center),1.0)))+radius;
    let uncertainty=(magnitude+vec2<f32>(1.0))*0.000003814697265625;
    let uv=p.xy*vec2<f32>(0.5,-0.5)+0.5;
    let pad=(radius+uncertainty)*0.5*f32(size)+guard;
    let lo=uv*f32(size)-pad;let hi=uv*f32(size)+pad;
    if !all(abs(lo)<vec2<f32>(1e25)) || !all(abs(hi)<vec2<f32>(1e25)) {return result;}
    result.kind=0u;
    if any(hi<vec2<f32>(0.0)) || any(lo>vec2<f32>(f32(size))) {return result;}
    let end=vec2<f32>(f32(size-1u));
    result.first=vec2<u32>(clamp(floor(lo),vec2<f32>(0.0),end))/64u;
    result.last=vec2<u32>(clamp(ceil(hi),vec2<f32>(0.0),end))/64u;
    result.kind=1u;return result;
}
