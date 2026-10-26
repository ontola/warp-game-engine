// Composed after the normal scene-lighting source and meadow_common.wgsl.
// Only two extra storage bindings: packed root/owner data and shared recipes.
@group(2) @binding(0) var<uniform> meadow:MeadowSettings;
@group(2) @binding(1) var<storage,read> meadow_pool:array<u32>;
@group(2) @binding(2) var<storage,read> meadow_recipe:array<vec4<f32>>;
@group(2) @binding(3) var<uniform> meadow_draw:vec4<u32>;
struct MeadowVertex {
    @builtin(position) position:vec4<f32>,
    @location(0) world:vec3<f32>,
    @location(1) previous_world:vec3<f32>,
    @location(2) normal:vec3<f32>,
    @location(3) color:vec4<f32>,
    @location(4) @interpolate(flat) opacity:f32,
};
fn meadow_read4(offset:u32)->vec4<f32> {
    return bitcast<vec4<f32>>(vec4<u32>(meadow_pool[offset],meadow_pool[offset+1u],meadow_pool[offset+2u],meadow_pool[offset+3u]));
}
fn meadow_source_owner(instance:u32,lod:u32,capacity:u32)->u32 {
    if lod>=3u || instance>=capacity || arrayLength(&meadow_pool)<capacity*31u {return 0xffffffffu;}
    return meadow_pool[capacity*MEADOW_PLANT_WORDS+lod*capacity+instance];
}
// One implementation serves ordinary raster vertices and reconstruction of
// an actually visible primitive. Recipe indices, habitat and wind stay exact.
fn meadow_source_vertex(vertex:u32,owner:u32,lod:u32,capacity:u32)->MeadowVertex {
    var out:MeadowVertex;out.position=vec4<f32>(2.0,2.0,2.0,1.0);
    if lod>=3u || owner>=capacity || arrayLength(&meadow_pool)<capacity*31u {return out;}
    let offset=owner*MEADOW_PLANT_WORDS;let root=meadow_read4(offset);let yaw=meadow_read4(offset+4u);
    let variant=meadow_pool[offset+10u];if variant>=3u {return out;}
    let mesh=bitcast<vec4<u32>>(meadow_recipe[variant*3u+lod]);
    if vertex>=mesh.y {return out;}
    let address=MEADOW_RECIPE_VERTICES+(mesh.x+vertex)*4u;
    if address+3u>=arrayLength(&meadow_recipe) {return out;}
    let point=meadow_recipe[address];
    let anchor=meadow_recipe[address+3u];
    let pivot_world=meadow_yaw(anchor.xyz,yaw.xy);
    if !meadow_road_anchor_allowed(meadow_pool[offset+8u],bitcast<f32>(meadow_pool[offset+9u]),
        pivot_world.xz,root.xz+pivot_world.xz,meadow.grid.z) {return out;}
    let normal=meadow_recipe[address+1u];let color=meadow_recipe[address+2u];
    let branch=min(u32(max(point.w,0.0)),3u);
    let sc=meadow_read4(offset+12u+branch*4u);
    let deformed=meadow_deform_source(point.xyz,normal.xyz,normal.xyz,anchor.xyz,root,yaw,sc,
        meadow.previous.yz,meadow.previous.w>=0.5);
    out.world=deformed.world;out.previous_world=deformed.previous_world;out.normal=deformed.normal;
    let pigment=meadow_unpack_tint(meadow_pool[offset+11u]);
    // A small, continuous blade pigment gradient survives source LOD cuts.
    // The actual shared scene lighting below still determines shade and sun.
    let height=clamp(point.y/max(meadow.shape.z,0.001),0.0,1.0);
    let blade_tint=mix(vec3<f32>(0.91,0.94,0.86),vec3<f32>(1.10,1.04,0.98),height);
    out.color=vec4<f32>(color.rgb*pigment*blade_tint*meadow.edits.rgb,color.w);
    out.opacity=1.0-smoothstep(meadow.shape.w,meadow.range.x,distance(root.xz,view.cam.xz));
    if meadow.support.z>0.5 {out.opacity=1.0;}
    out.position=view.view_proj*vec4<f32>(out.world,1.0);return out;
}
@vertex
fn meadow_vs(@builtin(vertex_index) vertex:u32,@builtin(instance_index) instance:u32)->MeadowVertex {
    let owner=meadow_source_owner(instance,meadow_draw.x,meadow_draw.y);
    return meadow_source_vertex(vertex,owner,meadow_draw.x,meadow_draw.y);
}
@fragment fn meadow_depth_fs(in:MeadowVertex) {
    // True source triangles have no photo alpha. The very same geometry and
    // opacity condition are used by the nearest-depth lighting pass.
    if in.opacity<=0.0 {discard;}
}
fn meadow_headlights(p:vec3<f32>,n:vec3<f32>,v:vec3<f32>,base:vec3<f32>,rough:f32)->vec3<f32> {
    let f0=vec3<f32>(0.04);
    let specular_weight=clamp(env_brdf(f0,rough,max(dot(n,v),1e-4)),vec3<f32>(0.0),vec3<f32>(1.0));
    let budget=clamp(base,vec3<f32>(0.0),vec3<f32>(1.0))*(1.0-specular_weight)/PI;
    var result=vec3<f32>(0.0);
    for(var index=0u;index<min(headlights.count.x,8u);index++) {
        let towards=headlights.lights[index].position_range.xyz-p;
        let bias=n*select(-1.0,1.0,dot(n,towards)>=0.0);
        let lamp=headlight_sample(index,p,bias);
        if all(lamp.irradiance==vec3<f32>(0.0)) {continue;}
        let ndl=dot(n,lamp.direction);
        result+=lamp.irradiance*(budget*(0.75*max(ndl,0.0)+0.25*max(-ndl,0.0))+ggx(n,v,lamp.direction,rough,f0));
    }
    return result;
}
fn meadow_color_with_geometric(in:MeadowVertex,geometric:vec3<f32>)->vec4<f32> {
    if in.opacity<=0.0 {discard;}
    let p=in.world;let v=meadow_safe_normal(view.cam.xyz-p);
    var n=meadow_safe_normal(in.normal);if dot(n,v)<0.0 {n=-n;}
    let geo_n=geometric*select(-1.0,1.0,dot(geometric,n)>=0.0);
    let base=clamp(in.color.rgb,vec3<f32>(0.0),vec3<f32>(1.0));let rough=clamp(in.color.w,0.04,1.0);
    let l=light.sun_dir.xyz;var shadow=1.0;
    if light.sun_dir.w>0.0 {
        shadow=mix(1.0,shadow_at_surface(p,n*sign(dot(n,l)+1e-4),geo_n,0.0),light.sun_color.w);
    }
    let sun_e=light.sun_color.rgb*light.sun_dir.w*shadow;
    let sky_vis=sky_visibility(p,n);let back_sky_vis=sky_visibility(p,-n);
    let refl=reflect(-v,n);let contact=contact_visibility(p+n*0.025,n,refl,0xffffffffu);
    let bounce=gi.bounce.rgb*(gi.bounce.w*sky_vis+light.sun_dir.w*max(light.sun_dir.y,0.0)*shadow);
    var reflection_vis=sky_vis;if rough<0.55 {reflection_vis=reflected_visibility(p,n,refl);}
    let front_ambient=(sky_irradiance(n)*sky_vis+bounce*(1.0-sky_vis))*contact.x;
    let back_ambient=(sky_irradiance(-n)*back_sky_vis+bounce*(1.0-back_sky_vis))*contact.x;
    let spec_contact=mix(contact.y,contact.x,smoothstep(0.08,0.6,rough));
    let env=mix(bounce/PI,environment(refl,rough,bounce),reflection_vis)*spec_contact;
    let response=thin_foliage_response(base,n,v,rough,1.0,sun_e,front_ambient,back_ambient,env);
    let direct=response.direct+meadow_headlights(p,n,v,base,rough);
    var rad=apply_distance_fog(direct+response.indirect+response.specular,distance(p,view.cam.xyz),-v);
    if light.sky.z>0.5 && light.sky.z<1.5 {rad=vec3<f32>(sky_vis)*6000.0;}
    else if light.sky.z>1.5 && light.sky.z<2.5 {rad=base*6000.0;}
    else if light.sky.z>2.5 && light.sky.z<3.5 {rad=direct;}
    else if light.sky.z>3.5 && light.sky.z<4.5 {rad=response.indirect;}
    else if light.sky.z>4.5 && light.sky.z<5.5 {rad=response.specular;}
    else if light.sky.z>5.5 && light.sky.z<6.5 {rad=vec3<f32>(contact.x)*6000.0;}
    else if light.sky.z>6.5 && light.sky.z<7.5 {rad=vec3<f32>(reflection_vis)*6000.0;}
    return vec4<f32>(rad*in.opacity,in.opacity);
}
fn meadow_color(in:MeadowVertex)->vec4<f32> {
    // Source geometry derivatives must be evaluated before any rejection.
    // Surface-cache fragments supply their sampled source normal instead:
    // derivatives of nearest atlas sample positions are not face geometry.
    let geometric=meadow_safe_normal(cross(dpdx(in.world),dpdy(in.world)));
    return meadow_color_with_geometric(in,geometric);
}
@fragment fn meadow_fs(in:MeadowVertex)->@location(0) vec4<f32> {return meadow_color(in);}
struct MeadowMotionOut {@location(0) color:vec4<f32>,@location(1) reprojection:vec4<f32>};
@fragment fn meadow_motion_fs(in:MeadowVertex)->MeadowMotionOut {
    var out:MeadowMotionOut;out.color=meadow_color(in);
    // The exact nearest source surface, before jitter removal/projection by
    // motion.wgsl. First-frame history has zero delta but still valid depth.
    out.reprojection=vec4<f32>(in.previous_world-in.world,distance(in.world,view.cam.xyz));return out;
}
