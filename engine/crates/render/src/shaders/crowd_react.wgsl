// Events arrive in simulation order. One workgroup cooperates on each nearest-hit
// reduction, then commits before the next event sees the updated people.
struct Event { a:vec4<f32>, b:vec4<f32>, tag:vec4<u32>, mood:vec4<f32> }
@group(1) @binding(0) var<storage,read> events:array<Event>;
struct Hit { seat:u32, event:u32, pad:vec2<u32>, position:vec4<f32> }
struct Results { count:atomic<u32>, pad0:u32, pad1:vec2<u32>, hits:array<Hit> }
@group(1) @binding(1) var<storage,read_write> results:Results;
const MISS:f32=1e30;
const LANES:u32=128u;
var<workgroup> distances:array<f32,128>;
var<workgroup> ids:array<u32,128>;
var<workgroup> consumed:atomic<u32>;

fn closest_point(a:vec3<f32>,b:vec3<f32>,p:vec3<f32>)->vec3<f32>{
    let delta=b-a;
    return a+delta*clamp(dot(p-a,delta)/max(dot(delta,delta),1e-12),0.,1.);
}
// First intersection with an upright finite cylinder, including a ray starting inside.
fn body_hit(a:vec3<f32>,dir:vec3<f32>,len:f32,base:vec3<f32>,height:f32,radius:f32)->f32{
    let p=a-base;
    var lo=0.; var hi=len;
    let side=dot(dir.xz,dir.xz); let c=dot(p.xz,p.xz)-radius*radius;
    if side<1e-10 { if c>0. {return MISS;} }
    else {
        let b=dot(dir.xz,p.xz); let disc=b*b-side*c;
        if disc<0. {return MISS;}
        lo=max(lo,(-b-sqrt(disc))/side); hi=min(hi,(-b+sqrt(disc))/side);
    }
    if abs(dir.y)<1e-7 {if p.y<0. || p.y>height {return MISS;}}
    else {
        let t0=-p.y/dir.y; let t1=(height-p.y)/dir.y;
        lo=max(lo,min(t0,t1)); hi=min(hi,max(t0,t1));
    }
    return select(MISS,lo,lo<=hi);
}
fn scare(i:u32,s:Seat,at:vec3<f32>,pos:vec3<f32>,range:f32,time:f32,blast:bool){
    let dist=distance(at,pos);
    if dist>=range {return;}
    var r=reactions[i];
    if blast {
        r=turn_from_blast(r,time,pos.xz-at.xz,s.pos_yaw.w,s.params.y,1.-dist/range);
    }
    // A second stimulus extends the same escape, including its direction.
    // Once walking home, start the next escape at the actual current offset.
    if r.goal==0. || time>=r.hold_end {
        let tangent=row_direction(s);
        let side=select(-1.,1.,dot(tangent,pos.xz-at.xz)>=0.);
        r.initial=displacement(r,time);
        r.goal=side*(1.-dist/range)*(1.4+2.6*s.params.y);
        r.start=time;
    }
    r.hold_end=max(r.hold_end,time+6.);
    reactions[i]=r;
}
fn die(i:u32,e:u32,p:vec3<f32>,bullet:vec2<u32>){
    reactions[i].dead=1u;
    reactions[i].bullet_lo=bullet.x; reactions[i].bullet_hi=bullet.y;
    let dst=atomicAdd(&results.count,1u);
    results.hits[dst]=Hit(i,e,vec2<u32>(0),vec4<f32>(p,1.));
}
@compute @workgroup_size(128)
fn react(@builtin(local_invocation_index) lane:u32){
    for(var k=0u;k<arrayLength(&events);k++){
        let e=events[k];
        let shot=e.tag.x==1u;
        let delta=e.b.xyz-e.a.xyz;
        let len=length(delta);let dir=delta/max(len,1e-8);
        var best=MISS;var winner=0xffffffffu;
        if lane==0u {atomicStore(&consumed,0u);}
        workgroupBarrier();
        for(var i=lane;i<u.counts.w;i+=LANES){
            let r=reactions[i];
            if r.dead!=0u {
                if shot && any(e.tag.yz!=vec2<u32>(0)) && all(e.tag.yz==vec2<u32>(r.bullet_lo,r.bullet_hi)){atomicOr(&consumed,1u);}
                continue;
            }
            if alive[i]==0u {continue;}
            let s=seats[i];
            // Most people lie nowhere near the event. Reject before animation.
            let near=closest_point(e.a.xyz,e.b.xyz,s.pos_yaw.xyz);
            if distance(near,s.pos_yaw.xyz)>e.mood.z+7. {continue;}
            let pose=body_pose(s,r,e.a.w,e.mood.xy);
            let h=pose.crouch*s.params.x;let center=pose.origin+vec3<f32>(0.,h*0.5,0.);
            let at=closest_point(e.a.xyz,e.b.xyz,center);
            if shot {
                let t=body_hit(e.a.xyz,dir,len,pose.origin,h,s.params.x*0.135);
                if t<best {best=t;winner=i;}
            } else if e.tag.x==2u && distance(vec3<f32>(center.x,clamp(at.y,pose.origin.y,pose.origin.y+h),center.z),at)<=e.b.w {
                die(i,k,center,vec2<u32>(0));continue;
            }
            if !shot {scare(i,s,at,center,e.mood.z,e.a.w,e.tag.x==2u);}
        }
        distances[lane]=best;ids[lane]=winner;
        workgroupBarrier();
        for(var step=LANES/2u;step>0u;step/=2u){
            if lane<step {
                let j=lane+step;
                if distances[j]<distances[lane] || (distances[j]==distances[lane] && ids[j]<ids[lane]){
                    distances[lane]=distances[j]; ids[lane]=ids[j];
                }
            }
            workgroupBarrier();
        }
        if lane==0u && shot && ids[0]!=0xffffffffu && atomicLoad(&consumed)==0u{
            die(ids[0],k,e.a.xyz+dir*distances[0],e.tag.yz);
        }
        storageBarrier();workgroupBarrier();
        if shot && atomicLoad(&consumed)==0u {
            let end=e.a.xyz+dir*min(len,distances[0]);
            for(var i=lane;i<u.counts.w;i+=LANES){
                if alive[i]==0u || reactions[i].dead!=0u {continue;}
                let s=seats[i];let near=closest_point(e.a.xyz,end,s.pos_yaw.xyz);
                if distance(near,s.pos_yaw.xyz)>e.mood.z+7. {continue;}
                let pose=body_pose(s,reactions[i],e.a.w,e.mood.xy);
                let center=pose.origin+vec3<f32>(0.,pose.crouch*s.params.x*0.5,0.);
                scare(i,s,closest_point(e.a.xyz,end,center),center,e.mood.z,e.a.w,false);
            }
        }
        storageBarrier();workgroupBarrier();
    }
}
