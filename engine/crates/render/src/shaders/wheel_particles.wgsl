struct ParticleUniform {
    output_vp:mat4x4<f32>,depth_vp:mat4x4<f32>,shadow_vp:mat4x4<f32>,near_vp:mat4x4<f32>,
    camera_time:vec4<f32>,right_near:vec4<f32>,up:vec4<f32>,sun_dir:vec4<f32>,sun_color:vec4<f32>,
    ambient:vec4<f32>,shadow:vec4<f32>,size:vec4<f32>,
};
struct Particle {position_radius:vec4<f32>,velocity_born:vec4<f32>,normal_life:vec4<f32>,color_kind:vec4<f32>,params:vec4<f32>};
@group(0) @binding(0) var<uniform> fx:ParticleUniform;
@group(0) @binding(1) var<storage,read> births:array<Particle>;
@group(0) @binding(2) var<storage,read> order:array<u32>;
@group(0) @binding(3) var scene_depth:texture_depth_2d;
@group(0) @binding(4) var far_shadow:texture_depth_2d;
@group(0) @binding(5) var near_shadow:texture_depth_2d;
@group(0) @binding(6) var shadow_sampler:sampler_comparison;
struct ParticleVertex {
    @builtin(position) position:vec4<f32>,@location(0) uv:vec2<f32>,@location(1) world:vec3<f32>,
    @location(2) normal:vec3<f32>,@location(3) @interpolate(flat) id:u32,
    @location(4) @interpolate(flat) sunlight:f32,@location(5) @interpolate(flat) age:f32,
};
fn particle_shadow(map:texture_depth_2d,vp:mat4x4<f32>,texel:f32,p:vec3<f32>)->f32 {
    let c=vp*vec4<f32>(p+fx.sun_dir.xyz*texel,1.0);let uv=c.xy*vec2<f32>(0.5,-0.5)+0.5;
    if any(uv<vec2<f32>(0.0))||any(uv>vec2<f32>(1.0))||c.z<0.0||c.z>1.0{return 1.0;}
    let d=vec2<f32>(0.35)/vec2<f32>(textureDimensions(map));
    return 0.5*(textureSampleCompareLevel(map,shadow_sampler,uv-d,c.z)+textureSampleCompareLevel(map,shadow_sampler,uv+d,c.z));
}
fn particle_sun(p:vec3<f32>)->f32 {
    var s=particle_shadow(far_shadow,fx.shadow_vp,fx.shadow.x,p);
    if fx.shadow.z>0.5 {
        let c=fx.near_vp*vec4<f32>(p,1.0);
        if c.z>=0.0&&c.z<=1.0{let weight=1.0-smoothstep(0.8,1.0,max(abs(c.x),abs(c.y)));
            if weight>0.0{s=mix(s,particle_shadow(near_shadow,fx.near_vp,fx.shadow.y,p),weight);}}
    }
    return mix(1.0,s,fx.sun_dir.w);
}
fn spin(v:vec3<f32>,angle:f32)->vec3<f32>{
    let a=vec3<f32>(0.4364358,0.8728716,0.2182179);let c=cos(angle);let s=sin(angle);
    return v*c+cross(a,v)*s+a*dot(a,v)*(1.0-c);
}
fn rock_corner(i:u32,seed:f32)->vec3<f32>{
    let p=array<vec3<f32>,6>(vec3<f32>(0.,1.,0.),vec3<f32>(0.,-1.,0.),vec3<f32>(1.,0.,0.),vec3<f32>(0.,0.,1.),vec3<f32>(-1.,0.,0.),vec3<f32>(0.,0.,-1.))[i];
    return p*vec3<f32>(0.75+0.22*sin(seed),0.66+0.15*cos(seed*1.7),0.9)*(0.85+0.15*sin(f32(i)*2.1+seed));
}
@vertex fn particle_vs(@builtin(vertex_index) v:u32,@builtin(instance_index) instance:u32)->ParticleVertex {
    let id=order[instance];let p=births[id];let age=max(0.0,fx.camera_time.w-p.velocity_born.w);let kind=u32(p.color_kind.w);
    let drag=p.params.z;let travel=(1.0-exp(-drag*age))/drag;
    var center=p.position_radius.xyz+p.velocity_born.xyz*travel-vec3<f32>(0.,p.params.y*(age/drag-travel/drag),0.);
    let angle=p.params.x+age*(7.0+2.0*sin(p.params.x));var offset=vec3<f32>(0.0);var normal=vec3<f32>(0.,1.,0.);var uv=vec2<f32>(0.0);
    let dust=kind==4u||kind==5u;let radius=p.position_radius.w*select(1.0,1.0+age*2.2,dust);
    if dust||kind==2u {
        let q=array<vec2<f32>,6>(vec2<f32>(-1.,-1.),vec2<f32>(1.,-1.),vec2<f32>(-1.,1.),vec2<f32>(-1.,1.),vec2<f32>(1.,-1.),vec2<f32>(1.,1.))[min(v,5u)];
        // Remaining vertices collapse to a point; only two dust/grass triangles.
        uv=select(vec2<f32>(0.0),q,v<6u);
        if dust {offset=(fx.right_near.xyz*uv.x+fx.up.xyz*uv.y)*radius;normal=normalize(fx.camera_time.xyz-center);}
        else {
            let width=(0.13+0.09*(1.0-uv.y))*uv.x;let bend=0.25*(uv.y*uv.y);
            offset=spin(vec3<f32>(width,uv.y,bend)*radius,angle);normal=spin(vec3<f32>(0.,0.,1.),angle);
        }
    } else {
        let face=v/3u;let side=face%4u;let a=rock_corner(select(0u,1u,face>=4u),p.params.x);
        let b=rock_corner(2u+side,p.params.x);let c=rock_corner(2u+(side+1u)%4u,p.params.x);
        offset=spin(array<vec3<f32>,3>(a,b,c)[v%3u]*radius,angle);
        normal=normalize(cross(b-a,c-a));normal*=select(-1.0,1.0,dot(normal,a+b+c)>0.0);normal=spin(normal,angle);
    }
    // Short-lived ballistic fragments disappear into their local contact plane.
    // This is not a rigid-body collision or a terrain collider query.
    if !dust&&dot(center-p.position_radius.xyz,p.normal_life.xyz)< -radius {center=vec3<f32>(0.0);offset=vec3<f32>(0.0);}
    let world=center+offset;
    return ParticleVertex(fx.output_vp*vec4<f32>(world,1.0),uv,world,normal,id,particle_sun(center),age);
}
@fragment fn particle_fs(input:ParticleVertex)->@location(0) vec4<f32>{
    let p=births[input.id];let kind=u32(p.color_kind.w);let dust=kind==4u||kind==5u;
    let clip=fx.depth_vp*vec4<f32>(input.world,1.0);let uv=clip.xy/clip.w*vec2<f32>(0.5,-0.5)+0.5;
    if clip.w<=0.0||any(uv<vec2<f32>(0.0))||any(uv>vec2<f32>(1.0)){discard;}
    let dims=vec2<i32>(textureDimensions(scene_depth));let pixel=clamp(vec2<i32>(uv*vec2<f32>(dims)),vec2<i32>(0),dims-1);
    let gap=fx.right_near.w/max(textureLoad(scene_depth,pixel,0),1e-9)-clip.w;if gap<=0.0{discard;}
    let lifetime=p.normal_life.w;let age=input.age;var alpha=p.params.w*(1.0-smoothstep(lifetime*0.65,lifetime,age));
    var n=normalize(input.normal);var light=vec3<f32>(0.0);
    // Debris is a real convex mesh. Reject back faces explicitly so opaque
    // fragments cannot paint their hidden side over their visible front side.
    if !dust&&kind!=2u&&dot(n,fx.camera_time.xyz-input.world)<=0.0{discard;}
    if dust {
        let q=input.uv;let seed=p.params.x;
        // Smooth overlapping lobes retain density through the puff interior;
        // every outside edge tends continuously to zero, never a square cutout.
        let lobe=0.84+0.12*sin(q.x*4.2+seed)*sin(q.y*4.9-seed+age);
        let radial=max(0.0,1.0-dot(q,q)/(lobe*lobe));
        let detail=0.75+0.25*sin(q.x*7.3+q.y*3.7+seed)*sin(q.y*6.1-q.x*2.9+seed*1.4);
        alpha*=radial*radial*detail*smoothstep(0.0,0.06,age)*clamp(gap/max(0.12,p.position_radius.w*2.0),0.0,1.0);
        // Low-order, single-scattering appearance. Existing sun shadows keep
        // a dust cloud under trees from receiving an unoccluded sun highlight.
        let direction=normalize(fx.camera_time.xyz-input.world);let phase=(0.8+0.2*pow(max(0.0,dot(-direction,fx.sun_dir.xyz)),3.0))/12.5663706;
        light=fx.ambient.rgb/3.14159265+fx.sun_color.rgb*phase*input.sunlight;
    } else {
        if kind==2u {n=faceForward(n,normalize(input.world-fx.camera_time.xyz),n);}
        let sky=0.4+0.6*max(0.0,n.y);
        light=(fx.ambient.rgb*sky+fx.sun_color.rgb*max(0.0,dot(n,fx.sun_dir.xyz))*input.sunlight)/3.14159265;
        alpha*=clamp(gap/max(0.005,p.position_radius.w*0.3),0.0,1.0);
    }
    if alpha<0.0001{discard;}
    return vec4<f32>(p.color_kind.rgb*light*alpha,alpha);
}
