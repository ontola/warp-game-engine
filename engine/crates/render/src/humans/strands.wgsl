// Evaluated from a vertex ID. Each segment uses two triangles.
// One periodic fitted field is shared by every head with the same cut.
struct GuideSample { xy:u32, z:u32, d0:u32, d1:u32, d2:u32, d3:u32 }
@group(0) @binding(8) var<storage,read> guides:array<GuideSample>;
fn guide_point(style:u32, col:u32, row:u32, w:vec4<f32>)->vec3<f32> {
    let v=guides[style*1024u+(col%64u)*16u+row];
    return vec3<f32>(0.,.9,0.)+.25*vec3<f32>(unpack2x16snorm(v.xy),unpack2x16snorm(v.z).x)
        +.055*(unpack4x8snorm(v.d0).xyz*w.x+unpack4x8snorm(v.d1).xyz*w.y
               +unpack4x8snorm(v.d2).xyz*w.z+unpack4x8snorm(v.d3).xyz*w.w);
}
fn groom_surface(style:u32, a:f32, h:f32, w:vec4<f32>)->vec3<f32> {
    let x=fract(a)*64.; let col=u32(x);
    let y=clamp(h,0.,.99999)*15.; let row=u32(y);
    let low=mix(guide_point(style,col,row,w),guide_point(style,col+1u,row,w),fract(x));
    let high=mix(guide_point(style,col,row+1u,w),guide_point(style,col+1u,row+1u,w),fract(x));
    return mix(low,high,fract(y));
}
fn strand_hash(id:u32)->f32 {
    var v=(id^(id>>16u))*0x7feb352du;
    v=(v^(v>>15u))*0x846ca68bu;
    return f32((v^(v>>16u))>>8u)/16777216.;
}
struct Fibre { p:vec3<f32>, normal:vec3<f32>, across:vec3<f32>, tangent:vec3<f32>, tint:f32, side:f32, radius:f32 }
// Interpolate neighbouring locks and their inner/outer guide layers first.
// A Hermite segment then supplies both the curve and its shading tangent.
fn bob_control(c:u32, u:f32, depth:f32, row:u32, weights:vec4<f32>)->vec3<f32> {
    let outer=mix(guide_point(1u,c,row,weights),guide_point(1u,c+1u,row,weights),u);
    let inner=mix(guide_point(1u,c,row+8u,weights),guide_point(1u,c+1u,row+8u,weights),u);
    return mix(outer,inner,depth);
}
fn bob_fibre(id:u32, t:f32, side:f32, seed:u32, weights:vec4<f32>, clock:f32)->Fibre {
    let salt=seed+151u*id;
    let panel=(id%2u)*32u;
    let angle=fract(.618033989*f32(id/2u)+strand_hash(seed));
    // A lock shares its bend and pigment. Neighbouring fine hairs approach
    // its centre towards the tip, leaving both dense locks and open space.
    let cell=u32(angle*47.);
    let lock=seed+cell*177u+panel*137u;
    let r=strand_hash(lock+71u);
    let centre=(f32(cell)+.23+.54*r)/47.;
    let a=f32(panel)+mix(angle,centre,t*t*(.48+.35*r))*30.999;
    let c=u32(a); let u=fract(a);
    let depth=pow(strand_hash(salt+1u),1.6);
    let length=.82+.10*strand_hash(lock+6u)+.08*strand_hash(salt+2u);
    let v=t*length*7.; let row=min(u32(v),6u); let f=v-f32(row);
    let p0=bob_control(c,u,depth,row-min(row,1u),weights);
    let p1=bob_control(c,u,depth,row,weights);
    let p2=bob_control(c,u,depth,row+1u,weights);
    let p3=bob_control(c,u,depth,min(row+2u,7u),weights);
    let b=.5*(p2-p0); let e=.5*(p3-p1);
    let curve_a=b+e-2.*(p2-p1); let curve_b=3.*(p2-p1)-2.*b-e;
    var p=((curve_a*f+curve_b)*f+b)*f+p1;
    let tangent=normalize((3.*curve_a*f+2.*curve_b)*f+b);
    let n=normalize(vec3<f32>(p.x,1.2*(p.y-.942),p.z-.002));
    let cross_axis=cross(tangent,n);
    let across=cross_axis*inverseSqrt(max(dot(cross_axis,cross_axis),1e-9));
    let phase=strand_hash(salt+4u)*6.283185;
    let fly=smoothstep(.91,1.,strand_hash(salt+14u));
    let roll=7.*t+2.*sin(.58*a)+.8*phase;
    let bend=6.5*t+6.283185*r;
    p+=n*(.0022*t*sin(bend)+.0010*t*sin(roll)+fly*.0065*t*t);
    p+=across*.0036*t*sin(bend+.7);
    p+=across*(fly*.0045*t*sin(6.*t+phase)+.0015*t*sin(roll));
    p+=across*(.00065*t*sin(5.*t+phase)+.00135*t*t*sin(clock*2.2+6.283185*a/64.));
    let radius=mix(.00009,.00020,strand_hash(salt+3u))*(1.-.92*t*t);
    let shade=(.68+.58*strand_hash(lock+8u))*(1.-.28*depth);
    return Fibre(p,n,across,tangent,shade*mix(.65,1.30,strand_hash(salt+5u)),side,radius);
}
fn grow_strand(vid:u32,style:u32,seed:u32,weights:vec4<f32>,clock:f32)->Fibre {
    let id=vid/54u; let corner=vid%6u;
    let upper=corner==1u || corner==4u || corner==5u;
    let side=select(-1.,1.,corner==2u || corner==3u || corner==5u);
    let t=(f32((vid/6u)%9u)+f32(upper))/9.;
    if style==1u {return bob_fibre(id,t,side,seed,weights,clock);}
    // Neighbours share a comb direction, with independent emergence and tips.
    let key=(id/5u)*149u+seed; let salt=id*151u+seed;
    let a0=fract(.618033989*f32(id/5u)+strand_hash(seed))+.009*(strand_hash(salt+37u)-.5);
    let start=.36+.59*strand_hash(key+2u)+.05*strand_hash(salt+11u);
    let loose=strand_hash(salt+8u);
    let end=max(-.022-.06*loose,start-(.24+.69*strand_hash(key+1u)+.15*strand_hash(salt+13u)));
    let h=mix(start,end,t);
    let dir=sin(6.283185*a0+1.3)+.25;
    let curl=6.283185*((1.7+.7*loose)*t+strand_hash(key+9u));
    let a=a0+t*dir*select(.116,.033,style==2u)+.006*t*(loose-.5)
        +select(0.,.013*sin(curl),style==2u);
    let theta=(a-.5)*6.283185;
    let radial=vec3<f32>(sin(theta),0.,cos(theta));
    let across=vec3<f32>(cos(theta),0.,-sin(theta));
    let normal=normalize(radial+vec3<f32>(0.,.08+2.4*max(h,0.)*max(h,0.),0.));
    var p=groom_surface(style,a,h,weights);
    let wave=sin(clock*2.7+f32(seed%157u)*.05+theta);
    p+=normal*(.0003+.0017*sin(3.14159*t)+.0019*loose*t*t
              +select(0.,.0013*cos(curl),style==2u));
    p+=across*(.0016*wave*t*t+.0013*sin(4.*t+theta)*t);
    p.y+=min(h,0.)*.075;
    // Fade tips as they enter the brow/eye opening. The fitted side hair
    // can still grow well below the jaw, where lateral clearance is ample.
    let clearance=max(p.y-.942,max(abs(p.x)-.041,.021-p.z));
    let radius=mix(.00014,.00027,strand_hash(salt+5u))*(1.-.90*t*t)*smoothstep(0.,.010,clearance);
    let tangent=normalize(cross(across,normal)+(.42*dir+select(0.,.65*cos(curl),style==2u))*across);
    return Fibre(p,normal,across,tangent,mix(.60,1.20,strand_hash(salt+6u)),side,radius);
}
