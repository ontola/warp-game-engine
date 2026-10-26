// Directional source-surface cache. This is NOT a volume or a new grass recipe.
// Rest coverage is an approximation under wind; representative surface points,
// real normals and previous positions use the same four branch poses as source.
const MEADOW_ATLAS_VIEW_STRIDE:u32=3u; // compact projection headers
const MEADOW_ATLAS_VARIANT_STRIDE:u32=24u*MEADOW_ATLAS_VIEW_STRIDE;
struct MeadowAggregateVertex {
    @builtin(position) position:vec4<f32>,
    @location(0) world:vec3<f32>,
    @location(1) @interpolate(flat) owner:u32,
};
@vertex fn meadow_aggregate_vs(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->MeadowAggregateVertex {
    var out:MeadowAggregateVertex;out.position=vec4<f32>(2.0,2.0,2.0,1.0);
    let capacity=meadow_draw.y;if instance>=capacity {return out;}
    let owner=meadow_pool[capacity*MEADOW_PLANT_WORDS+instance];if owner>=capacity {return out;}
    let root=meadow_read4(owner*MEADOW_PLANT_WORDS);let yaw=meadow_read4(owner*MEADOW_PLANT_WORDS+4u);
    let variant=meadow_pool[owner*MEADOW_PLANT_WORDS+10u];
    var angle=0.0;for(var branch=0u;branch<4u;branch++){angle=max(angle,abs(meadow_recipe[9u+variant*4u+branch].w));}
    let bound=bitcast<u32>(meadow.support.w)+216u+variant*4u;
    let margin=meadow.support.y*root.w*angle*1.5;
    let lower=meadow_recipe[bound].xyz+meadow_recipe[bound+2u].xyz*root.w-vec3<f32>(margin);
    let upper=meadow_recipe[bound+1u].xyz+meadow_recipe[bound+3u].xyz*root.w+vec3<f32>(margin);
    let toward=select(meadow_safe_normal(view.cam.xyz-root.xyz),meadow_safe_normal((view.inv_view_proj*vec4<f32>(0.0,0.0,1.0,0.0)).xyz),view.params.z>0.5);
    var right=meadow_safe_normal(cross(vec3<f32>(0.0,1.0,0.0),toward));
    if abs(toward.y)>0.999 {right=vec3<f32>(1.0,0.0,0.0);}
    let up=cross(toward,right);
    let camera_distance=distance(root.xyz,view.cam.xyz);
    var low=vec2<f32>(1e30);var high=vec2<f32>(-1e30);
    for(var k=0u;k<8u;k++) {
        let local=vec3<f32>(select(lower.x,upper.x,(k&1u)!=0u),select(lower.y,upper.y,(k&2u)!=0u),select(lower.z,upper.z,(k&4u)!=0u));
        let world=meadow_ground_shear(meadow_yaw(local,yaw.xy),yaw.zw);
        let ratio=select(camera_distance/max(camera_distance-dot(world,toward),1e-4),1.0,view.params.z>0.5);
        let projected=vec2<f32>(dot(world,right),dot(world,up))*ratio;low=min(low,projected);high=max(high,projected);
    }
    let corners=array<vec2<f32>,6>(vec2<f32>(-1.0,-1.0),vec2<f32>(1.0,-1.0),vec2<f32>(-1.0,1.0),
        vec2<f32>(-1.0,1.0),vec2<f32>(1.0,-1.0),vec2<f32>(1.0,1.0));
    let pixel_pad=select(camera_distance,1.0,view.params.z>0.5)/max(view.cam.w,1.0);
    let extent=mix(low-vec2<f32>(pixel_pad),high+vec2<f32>(pixel_pad),corners[vertex]*0.5+0.5);
    out.world=root.xyz+right*extent.x+up*extent.y;
    out.owner=owner;out.position=view.view_proj*vec4<f32>(out.world,1.0);return out;
}
struct MeadowSurface {point:vec4<f32>,normal:vec4<f32>,color:vec4<f32>,anchor:vec4<f32>};
@group(2) @binding(4) var meadow_atlas_material:texture_2d_array<f32>;
@group(2) @binding(5) var meadow_atlas_surface:texture_2d_array<f32>;
@group(2) @binding(6) var meadow_atlas_sampler:sampler;
fn meadow_atlas_sample(variant:u32,direction:u32,local_origin:vec3<f32>,local_ray:vec3<f32>,footprint:f32)->MeadowSurface {
    var result:MeadowSurface;
    let base=bitcast<u32>(meadow.support.w)+variant*72u+direction*3u;
    let right=meadow_recipe[base];let up=meadow_recipe[base+1u];let toward=meadow_recipe[base+2u];
    let center=vec3<f32>(0.0,toward.w,0.0);let denominator=dot(local_ray,toward.xyz);
    if abs(denominator)<1e-5 {return result;}
    let p=local_origin+local_ray*(dot(center-local_origin,toward.xyz)/denominator);
    let uv=vec2<f32>(dot(p-center,right.xyz)/max(right.w,1e-5),dot(p-center,up.xyz)/max(up.w,1e-5))*0.5+0.5;
    if any(uv<vec2<f32>(0.0)) || any(uv>vec2<f32>(1.0)) {return result;}
    let mip_value=clamp(log2(max(1.0,footprint*32.0/(2.0*min(right.w,up.w)))),0.0,5.0);
    let first_mip=u32(floor(mip_value));let fraction=fract(mip_value);
    let layer=i32((variant*24u+direction)*2u);
    var color=vec4<f32>(0.0);var normals=vec4<f32>(0.0);
    for(var level=0u;level<2u;level++) {
        let mip=min(first_mip+level,5u);let weight=select(1.0-fraction,fraction,level!=0u);
        if weight<=0.0 {continue;}
        let size=f32(32u>>mip);
        // Clamp-to-edge hardware filtering times the exact surviving support
        // fraction is equivalent to zero border for coordinates inside [0,1].
        let edge=clamp(uv*size+0.5,vec2<f32>(0.0),vec2<f32>(1.0))*clamp((1.0-uv)*size+0.5,vec2<f32>(0.0),vec2<f32>(1.0));
        let w=weight*edge.x*edge.y;
        color+=textureSampleLevel(meadow_atlas_material,meadow_atlas_sampler,uv,layer,f32(mip))*w;
        normals+=textureSampleLevel(meadow_atlas_material,meadow_atlas_sampler,uv,layer+1,f32(mip))*w;
    }
    if color.w>0.0 {result.color=vec4<f32>(color.rgb/color.w,color.w);result.normal=normals/color.w;}
    // The caller selects one direction before fetching discrete point/anchor.
    result.point=vec4<f32>(uv,mip_value,f32(layer));return result;
}
fn meadow_atlas_representative(surface:MeadowSurface)->MeadowSurface {
    var result=surface;let uv=surface.point.xy;let mip_value=surface.point.z;let layer=i32(surface.point.w);
    let first_mip=u32(floor(mip_value));let fraction=fract(mip_value);var strongest=0.0;
    for(var level=0u;level<2u;level++) {
        let mip=min(first_mip+level,5u);let weight=select(1.0-fraction,fraction,level!=0u);
        if weight<=0.0 {continue;}
        let size=32u>>mip;let q=uv*f32(size)-0.5;let first=vec2<i32>(floor(q));let f=fract(q);
        for(var y=0u;y<2u;y++){for(var x=0u;x<2u;x++) {
            let xy=first+vec2<i32>(i32(x),i32(y));if any(xy<vec2<i32>(0)) || any(xy>=vec2<i32>(i32(size))) {continue;}
            let coverage=textureLoad(meadow_atlas_material,xy,layer,i32(mip)).w;
            let w=coverage*weight*select(1.0-f.x,f.x,x!=0u)*select(1.0-f.y,f.y,y!=0u);
            if w>strongest {strongest=w;result.point=textureLoad(meadow_atlas_surface,xy,layer,i32(mip));result.anchor=textureLoad(meadow_atlas_surface,xy,layer+1,i32(mip));}
        }}
    }
    return result;
}
struct MeadowAggregateSurface {vertex:MeadowVertex,depth:f32};
fn meadow_aggregate_surface(in:MeadowAggregateVertex)->MeadowAggregateSurface {
    let offset=in.owner*MEADOW_PLANT_WORDS;let root=meadow_read4(offset);let yaw=meadow_read4(offset+4u);
    let variant=meadow_pool[offset+10u];let inverse_yaw=vec2<f32>(yaw.x,-yaw.y);
    var ray_origin=view.cam.xyz;
    var world_ray=meadow_safe_normal(in.world-view.cam.xyz);
    if view.params.z>0.5 {
        let xy=in.position.xy/view.size.xy*vec2<f32>(2.0,-2.0)+vec2<f32>(-1.0,1.0);
        let near=view.inv_view_proj*vec4<f32>(xy,1.0,1.0);
        let far=view.inv_view_proj*vec4<f32>(xy,0.0,1.0);
        ray_origin=near.xyz/near.w;world_ray=meadow_safe_normal(far.xyz/far.w-ray_origin);
    }
    let unshear_origin=meadow_ground_shear(ray_origin-root.xyz,-yaw.zw);
    let unshear_ray=meadow_ground_shear(world_ray,-yaw.zw);
    // Fixed basal anchors are not grown. The rest projection uses patch scale
    // only for source height; representative point reconstruction below keeps
    // the exact anchor and branch transform. This bounds the approximation.
    let local_origin=meadow_yaw(unshear_origin,inverse_yaw)/vec3<f32>(1.0,root.w,1.0);
    let local_ray=meadow_yaw(unshear_ray,inverse_yaw)/vec3<f32>(1.0,root.w,1.0);
    let toward=meadow_safe_normal(-local_ray);
    let az=fract(atan2(toward.x,toward.z)/MEADOW_TAU+1.0)*8.0;
    let elevation=clamp(asin(clamp(toward.y,-1.0,1.0))/(MEADOW_TAU/12.0),0.0,2.0);
    let az0=u32(floor(az))%8u;let el0=min(u32(floor(elevation)),1u);
    let azf=fract(az);let elf=clamp(elevation-f32(el0),0.0,1.0);
    let footprint=select(distance(root.xyz,view.cam.xyz),1.0,view.params.z>0.5)/max(view.cam.w,1.0)/max(root.w,0.1);
    var sample:MeadowSurface;var total=0.0;var strongest=0.0;var rgb=vec3<f32>(0.0);
    for(var e=0u;e<2u;e++){for(var a=0u;a<2u;a++){
        let s=meadow_atlas_sample(variant,(el0+e)*8u+(az0+a)%8u,local_origin,local_ray,footprint);
        let weight=select(1.0-azf,azf,a!=0u)*select(1.0-elf,elf,e!=0u);let covered=weight*s.color.w;
        rgb+=s.color.rgb*covered;sample.normal+=s.normal*covered;total+=covered;
        if covered>strongest {strongest=covered;sample.point=s.point;sample.anchor=s.anchor;}
    }}
    if total<=0.001 {discard;}
    sample=meadow_atlas_representative(sample);
    sample.color=vec4<f32>(rgb/total,total);sample.normal/=total;
    let anchor=meadow_yaw(sample.anchor.xyz,yaw.xy);
    if !meadow_road_anchor_allowed(meadow_pool[offset+8u],bitcast<f32>(meadow_pool[offset+9u]),anchor.xz,root.xz+anchor.xz,meadow.grid.z){discard;}
    let branch=min(u32(max(sample.point.w,0.0)),3u);let sc=meadow_read4(offset+12u+branch*4u);let axis=meadow_axis(meadow.previous.yz);
    let relative=meadow_yaw((sample.point.xyz-sample.anchor.xyz)*root.w,yaw.xy);
    var result:MeadowAggregateSurface;
    result.vertex.world=root.xyz+meadow_ground_shear(anchor+meadow_rotate(relative,axis,sc.xy),yaw.zw);
    result.vertex.previous_world=root.xyz+meadow_ground_shear(anchor+meadow_rotate(relative,axis,sc.zw),yaw.zw);
    // A representative atlas texel is not exactly on the current pixel ray.
    // Project it onto that ray and preserve its actual pose displacement so
    // stored depth, fog distance and motion reconstruction describe one point.
    let delta=result.vertex.previous_world-result.vertex.world;
    let ray_distance=dot(result.vertex.world-ray_origin,world_ray);
    if ray_distance<=0.0 {discard;}
    result.vertex.world=ray_origin+world_ray*ray_distance;
    result.vertex.previous_world=result.vertex.world+delta;
    if meadow.previous.w<0.5 {result.vertex.previous_world=result.vertex.world;}
    result.vertex.normal=meadow_ground_normal(meadow_rotate(meadow_yaw(sample.normal.xyz,yaw.xy),axis,sc.xy),yaw.zw);
    let height=clamp(sample.point.y/max(meadow.shape.z,0.001),0.0,1.0);
    let tint=mix(vec3<f32>(0.91,0.94,0.86),vec3<f32>(1.10,1.04,0.98),height)*meadow_unpack_tint(meadow_pool[offset+11u]);
    result.vertex.color=vec4<f32>(sample.color.rgb*tint*meadow.edits.rgb,sample.normal.w);
    // Outer distance fade is applied once to the resolved union. Fading every
    // fragment before product revealage would keep dense stacks opaque.
    result.vertex.opacity=total;
    let clip=view.view_proj*vec4<f32>(result.vertex.world,1.0);result.depth=clip.z/clip.w;
    if clip.w<=0.0 || result.depth<=0.0 || result.depth>1.0 {discard;}
    return result;
}
@fragment fn meadow_aggregate_depth_fs(in:MeadowAggregateVertex)->@builtin(frag_depth) f32 {
    return meadow_aggregate_surface(in).depth;
}
struct MeadowAggregateColor {@location(0) color:vec4<f32>,@builtin(frag_depth) depth:f32};
struct MeadowAggregateAccumulation {@location(0) color:vec4<f32>,@location(1) normal:vec4<f32>,@location(2) reveal:vec4<f32>,@builtin(frag_depth) depth:f32};
@fragment fn meadow_aggregate_fs(in:MeadowAggregateVertex)->MeadowAggregateAccumulation {
    let s=meadow_aggregate_surface(in);var out:MeadowAggregateAccumulation;
    let alpha=s.vertex.opacity;let viewer=select(meadow_safe_normal(view.cam.xyz-s.vertex.world),meadow_safe_normal((view.inv_view_proj*vec4<f32>(0.0,0.0,1.0,0.0)).xyz),view.params.z>0.5);
    var normal=meadow_safe_normal(s.vertex.normal);if dot(normal,viewer)<0.0 {normal=-normal;}
    // Coverage-premultiplied material moments only. Full scene illumination
    // happens once after resolving overlap, never per overlapping patch.
    out.color=vec4<f32>(s.vertex.color.rgb*alpha,alpha);
    out.normal=vec4<f32>(normal*alpha,s.vertex.color.w*alpha);
    out.reveal=vec4<f32>(alpha);out.depth=s.depth;return out;
}
struct MeadowAggregateMotion {@location(0) reprojection:vec4<f32>,@builtin(frag_depth) depth:f32};
@fragment fn meadow_aggregate_depth_motion_fs(in:MeadowAggregateVertex)->MeadowAggregateMotion {
    let s=meadow_aggregate_surface(in);var out:MeadowAggregateMotion;out.depth=s.depth;
    out.reprojection=vec4<f32>(s.vertex.previous_world-s.vertex.world,distance(s.vertex.world,view.cam.xyz));return out;
}
