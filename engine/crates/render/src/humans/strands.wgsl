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
fn grow_strand(vid:u32,style:u32,seed:u32,weights:vec4<f32>,clock:f32)->Fibre {
    let id=vid/54u; let corner=vid%6u;
    let upper=corner==1u || corner==4u || corner==5u;
    let side=select(-1.,1.,corner==2u || corner==3u || corner==5u);
    let t=(f32((vid/6u)%9u)+f32(upper))/9.;
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
    // The bob's lifted fringe must follow the same deformation as its base.
    if style==1u {
        let f=clamp((p.z-.033)/.027,0.,1.)*clamp(1.-abs(p.x-.006)/.052,0.,1.)*clamp((.974-p.y)/.032,0.,1.);
        p+=vec3<f32>(-.012,.030,0.)*f;
        let brow=smoothstep(.032,.059,p.z)*(1.-smoothstep(.023,.057,abs(p.x)));
        p.y+=brow*max(0.,.955-p.y);

    }
    let wave=sin(clock*2.7+f32(seed%157u)*.05+theta);
    p+=normal*(.0003+.0017*sin(3.14159*t)+.0019*loose*t*t
              +select(0.,.0013*cos(curl),style==2u));
    p+=across*(.0016*wave*t*t+.0013*sin(4.*t+theta)*t);
    p.y+=min(h,0.)*select(.075,.15,style==1u);
    // Fade tips as they enter the brow/eye opening. The fitted side hair
    // can still grow well below the jaw, where lateral clearance is ample.
    let clearance=max(p.y-.944,max(abs(p.x)-.039,.023-p.z));
    let radius=mix(.00014,.00027,strand_hash(salt+5u))*(1.-.90*t*t)*smoothstep(0.,.007,clearance);
    let tangent=normalize(cross(across,normal)+(.42*dir+select(0.,.65*cos(curl),style==2u))*across);
    return Fibre(p,normal,across,tangent,mix(.60,1.20,strand_hash(salt+6u)),side,radius);
}
