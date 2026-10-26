// Original GPU consumer for the CPU transport reference. No ray generation,
// screen-space lighting, or game-material integration occurs in this shader.
struct CacheParameters { values:vec4<f32>, counts:vec4<u32> }
struct CacheProbe { position_spacing:vec4<f32>, irradiance_sh:array<vec4<f32>,9>, metadata:vec4<u32> }
struct CacheQuery { position:vec4<f32>, normal:vec4<f32> }
struct CacheSample { irradiance_known:vec4<f32>, visibility_count:vec4<f32> }
@group(0) @binding(0) var<uniform> cache_parameters:CacheParameters;
@group(0) @binding(1) var<storage,read> cache_probes:array<CacheProbe>;
@group(0) @binding(2) var<storage,read> cache_moments:array<vec4<f32>>;
@group(0) @binding(3) var<storage,read> cache_keys:array<vec4<i32>>;
@group(0) @binding(4) var<storage,read> cache_queries:array<CacheQuery>;
@group(0) @binding(5) var<storage,read_write> cache_samples:array<CacheSample>;
@group(0) @binding(6) var cache_color:texture_storage_2d<rgba8unorm,write>;
@group(0) @binding(7) var cache_validity:texture_storage_2d<rgba8unorm,write>;

fn cache_key_less(a:vec3<i32>,b:vec3<i32>)->bool {
    if a.x!=b.x{return a.x<b.x;}if a.y!=b.y{return a.y<b.y;}return a.z<b.z;
}
fn cache_find(key:vec3<i32>)->u32 {
    // CPU BTreeMap order is lexicographic x,y,z. At most2048 keys =>12 steps.
    var low=0u;var high=cache_parameters.counts.x;
    for(var iteration=0u;iteration<12u;iteration++){
        if low>=high{break;}let middle=(low+high)/2u;let found=cache_keys[middle].xyz;
        if cache_key_less(found,key){low=middle+1u;}else{high=middle;}
    }
    if low<cache_parameters.counts.x {if all(cache_keys[low].xyz==key){return low;}}
    return 0xffffffffu;
}
fn cache_irradiance(index:u32,n:vec3<f32>)->vec3<f32> {
    let basis=array<f32,9>(0.2820948,0.48860252*n.y,0.48860252*n.z,0.48860252*n.x,
        1.0925485*n.x*n.y,1.0925485*n.y*n.z,0.31539157*(3.0*n.z*n.z-1.0),
        1.0925485*n.x*n.z,0.54627424*(n.x*n.x-n.y*n.y));
    var irradiance=vec3<f32>(0.0);
    for(var i=0u;i<9u;i++){irradiance+=cache_probes[index].irradiance_sh[i].xyz*basis[i];}
    return max(irradiance,vec3<f32>(0.0));
}
fn cache_moment(index:u32,d:vec3<f32>)->vec3<f32>{
    let a=abs(d);var face=0u;var uv=vec2<f32>(0.0);
    if a.x>=a.y && a.x>=a.z {
        if d.x>=0.0{face=0u;uv=vec2<f32>(-d.z,d.y)/a.x;}else{face=1u;uv=vec2<f32>(d.z,d.y)/a.x;}
    } else if a.y>=a.z {
        if d.y>=0.0{face=2u;uv=vec2<f32>(d.x,-d.z)/a.y;}else{face=3u;uv=vec2<f32>(d.x,d.z)/a.y;}
    } else {
        if d.z>=0.0{face=4u;uv=vec2<f32>(d.x,d.y)/a.z;}else{face=5u;uv=vec2<f32>(-d.x,d.y)/a.z;}
    }
    let q=clamp((uv+1.0)*2.0-0.5,vec2<f32>(0.0),vec2<f32>(3.0));
    let base=vec2<u32>(floor(q));let t=fract(q);var result=vec3<f32>(0.0);
    for(var y=0u;y<2u;y++){for(var x=0u;x<2u;x++){
        let weight=select(1.0-t.x,t.x,x==1u)*select(1.0-t.y,t.y,y==1u);
        let address=cache_probes[index].metadata.z+face*16u+min(base.y+y,3u)*4u+min(base.x+x,3u);
        result+=cache_moments[address].xyz*weight;
    }}
    return result;
}
fn cache_visibility(moment:vec3<f32>,distance:f32)->f32{
    if moment.z<0.999{return 0.0;}
    if distance<=moment.x+cache_parameters.values.y{return 1.0;}
    let squared_mean=moment.x*moment.x;let measured_variance=moment.y-squared_mean;
    let rounding=8.0*1.1920928955078125e-7*max(abs(moment.y),squared_mean);
    let variance=select(0.0,measured_variance,measured_variance>rounding);let delta=distance-moment.x;
    return variance/max(variance+delta*delta,1e-12);
}
fn cache_evaluate(position:vec3<f32>,normal:vec3<f32>)->CacheSample {
    let grid=position/cache_parameters.values.x;let length2=dot(normal,normal);
    if !all(abs(grid)<=vec3<f32>(16777200.0)) || !(length2>=1e-12){return CacheSample(vec4<f32>(0.0),vec4<f32>(0.0));}
    let base=vec3<i32>(floor(grid));let t=fract(grid);let n=normal*inverseSqrt(length2);
    var irradiance=vec3<f32>(0.0);var known=0.0;var visible=0.0;var count=0u;
    for(var z=0u;z<2u;z++){for(var y=0u;y<2u;y++){for(var x=0u;x<2u;x++){
        let weight=select(1.0-t.x,t.x,x==1u)*select(1.0-t.y,t.y,y==1u)*select(1.0-t.z,t.z,z==1u);
        if weight<=0.0{continue;}
        let key=base+vec3<i32>(i32(x),i32(y),i32(z));let index=cache_find(key);
        if index==0xffffffffu{continue;}if cache_probes[index].metadata.x==0u{continue;}
        known+=weight;
        let delta=position-cache_probes[index].position_spacing.xyz;let distance=length(delta);var visibility=1.0;
        if distance>=1e-5{visibility=cache_visibility(cache_moment(index,delta/distance),distance);}
        if visibility<=0.0{continue;}
        let w=weight*visibility;irradiance+=cache_irradiance(index,n)*w;visible+=w;count++;
    }}}
    if visible>1e-8{irradiance*=known/visible;}else{irradiance=vec3<f32>(0.0);}
    return CacheSample(vec4<f32>(irradiance,known),vec4<f32>(visible,f32(count),0.0,0.0));
}
fn cache_srgb(linear:vec3<f32>)->vec3<f32>{
    return select(1.055*pow(max(linear,vec3<f32>(0.0)),vec3<f32>(1.0/2.4))-0.055,linear*12.92,linear<=vec3<f32>(0.0031308));
}
@compute @workgroup_size(8,8)
fn sample_cache(@builtin(global_invocation_id) pixel:vec3<u32>){
    if pixel.x>=cache_parameters.counts.z || pixel.y>=cache_parameters.counts.w{return;}
    let index=pixel.y*cache_parameters.counts.z+pixel.x;
    let query=cache_queries[index];let result=cache_evaluate(query.position.xyz,query.normal.xyz);cache_samples[index]=result;
    // White diffuse receiver Lo=E/pi; fixed exposure + Reinhard only for PNG.
    // Raw numerical acceptance uses unclipped irradiance in cache_samples.
    let radiance=result.irradiance_known.xyz*(cache_parameters.values.z/3.141592653589793);
    textureStore(cache_color,vec2<i32>(pixel.xy),vec4<f32>(cache_srgb(radiance/(1.0+radiance)),1.0));
    // Red=known coverage, green=visible weight, blue=missing/invalid coverage.
    textureStore(cache_validity,vec2<i32>(pixel.xy),vec4<f32>(result.irradiance_known.w,result.visibility_count.x,1.0-result.irradiance_known.w,1.0));
}
