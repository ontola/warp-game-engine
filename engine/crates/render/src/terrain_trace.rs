//! Camera-independent terrain visibility, using a conservative min/max
//! hierarchy and exact native heightfield triangles. This is NOT a mesh SDF.
//! A finite clear segment does not certify sky outside the loaded world.
use crate::scene::Heightfield;
use glam::{DVec3,Vec3};

pub const DEFAULT_NODE_BUDGET:u32=256;
pub const MAX_NODE_BUDGET:u32=4096;
pub const MAX_LEVELS:usize=16;
pub const MAX_BASE_CELLS:usize=16*1024*1024;
pub const UNKNOWN_HEIGHT:f32=-1.0e8;
pub const SHADER:&str=include_str!("shaders/terrain_trace.wgsl");
const UNKNOWN_RANGE:[f32;2]=[-1.0e30,1.0e30];
const EMPTY_RANGE:[f32;2]=[1.0e30,-1.0e30];
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
#[repr(u32)]
pub enum Coverage {UnknownOutside=0,CompleteLoadedTerrain=1}

#[derive(Clone,Copy,Debug)]
pub struct Ray {pub origin:Vec3,pub direction:Vec3,pub t_min:f32,pub t_max:f32}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
#[repr(u32)]
pub enum Status {Clear=0,Hit=1,Unknown=2,AboveTerrainBounds=3}
#[derive(Clone,Copy,Debug,PartialEq,Eq)]
#[repr(u32)]
pub enum Reason {None=0,OutsideCoverage=1,UnknownHeight=2,Budget=3,InvalidRay=4,Stack=5}
#[derive(Clone,Copy,Debug)]
pub struct TraceResult {
    pub status:Status,pub reason:Reason,pub distance:f32,pub position:Vec3,pub normal:Vec3,
    pub visited_nodes:u32,pub triangle_tests:u32,
}
#[derive(Clone,Debug)]
pub struct Level {pub width:u32,pub height:u32,pub bounds:Vec<[f32;2]>}
pub struct Hierarchy {pub field:Heightfield,pub levels:Vec<Level>,pub coverage:Coverage}

fn known(h:f32)->bool {h.is_finite()&&h>UNKNOWN_HEIGHT&&h.abs()<1e30}
fn triangle_empty(h:[f32;3],policy:Coverage)->bool {
    policy==Coverage::CompleteLoadedTerrain&&h.iter().all(|v|v.is_finite()&&v.abs()<1e30)&&h.iter().any(|v|*v<=UNKNOWN_HEIGHT)
}
pub(crate) fn dimensions(hf:&Heightfield)->Result<(u32,u32),String>{
    if hf.w<2||hf.h<2||!hf.cell.is_finite()||hf.cell<=0.||!hf.origin.iter().all(|v|v.is_finite())
        ||(hf.w as usize).checked_mul(hf.h as usize)!=Some(hf.heights.len()) {
        return Err("invalid terrain trace heightfield".into());
    }
    if !(hf.origin[0]+(hf.w-1)as f32*hf.cell).is_finite()
        || !(hf.origin[1]+(hf.h-1)as f32*hf.cell).is_finite(){
        return Err("terrain trace world extent is not finite".into());
    }
    let w=(hf.w-1).checked_next_power_of_two().ok_or("terrain trace width overflow")?;
    let h=(hf.h-1).checked_next_power_of_two().ok_or("terrain trace height overflow")?;
    if w.max(h).ilog2() as usize>=MAX_LEVELS || (w as usize).checked_mul(h as usize).is_none_or(|n|n>MAX_BASE_CELLS) {
        return Err("terrain trace exceeds bounded hierarchy allocation".into());
    }
    Ok((w,h))
}
impl Hierarchy {
    pub fn build(hf:&Heightfield)->Result<Self,String>{
        Self::with_coverage(hf,Coverage::UnknownOutside)
    }
    pub fn with_coverage(hf:&Heightfield,coverage:Coverage)->Result<Self,String>{
        let (w,h)=dimensions(hf)?;let mut base=vec![EMPTY_RANGE;(w*h)as usize];
        for z in 0..hf.h-1 {for x in 0..hf.w-1 {let k=(z*hf.w+x)as usize;
            let corners=[hf.heights[k],hf.heights[k+1],hf.heights[k+hf.w as usize],hf.heights[k+hf.w as usize+1]];
            let mut range=EMPTY_RANGE;
            for tri in [[corners[0],corners[1],corners[2]],[corners[1],corners[2],corners[3]]] {
                if triangle_empty(tri,coverage){continue;}
                if !tri.iter().copied().all(known){range=UNKNOWN_RANGE;break;}
                for h in tri{range[0]=range[0].min(h);range[1]=range[1].max(h);}
            }
            base[(z*w+x)as usize]=range;
        }}
        let mut levels=vec![Level{width:w,height:h,bounds:base}];
        while levels.last().unwrap().width>1||levels.last().unwrap().height>1 {
            let p=levels.last().unwrap();let w=(p.width/2).max(1);let h=(p.height/2).max(1);
            let mut bounds=vec![[f32::INFINITY,f32::NEG_INFINITY];(w*h)as usize];
            for z in 0..h{for x in 0..w{let b=&mut bounds[(z*w+x)as usize];
                for dz in 0..2{for dx in 0..2{let sx=x*2+dx;let sz=z*2+dz;if sx<p.width&&sz<p.height{
                    let v=p.bounds[(sz*p.width+sx)as usize];b[0]=b[0].min(v[0]);b[1]=b[1].max(v[1]);
                }}}
            }}levels.push(Level{width:w,height:h,bounds});
        }
        Ok(Self{field:hf.clone(),levels,coverage})
    }
    pub fn storage_bytes(&self)->usize {self.levels.iter().map(|l|l.bounds.len()*8).sum::<usize>()+self.field.heights.len()*4}
    pub fn trace(&self,ray:Ray,node_budget:u32)->TraceResult {
        let mut result=TraceResult{status:Status::Unknown,reason:Reason::InvalidRay,distance:ray.t_min,position:ray.origin,normal:Vec3::ZERO,visited_nodes:0,triangle_tests:0};
        let length2=ray.direction.length_squared();
        if !ray.origin.is_finite()||ray.origin.abs().max_element()>=1e30||!ray.direction.is_finite()
            ||ray.direction.abs().max_element()>=1e30||!length2.is_finite()||length2<=1e-20
            ||!ray.t_min.is_finite()||!ray.t_max.is_finite()||ray.t_max>=1e30||ray.t_min<0.||ray.t_max<ray.t_min{return result;}
        let p=ray.origin.as_dvec3();let d=ray.direction.as_dvec3().normalize();let start=ray.t_min as f64;let limit=ray.t_max as f64;
        let domain_min=[self.field.origin[0]as f64,self.field.origin[1]as f64];
        let domain_max=[domain_min[0]+(self.field.w-1)as f64*self.field.cell as f64,domain_min[1]+(self.field.h-1)as f64*self.field.cell as f64];
        let at=p+d*start;
        if self.coverage==Coverage::UnknownOutside&&(at.x<domain_min[0]||at.x>domain_max[0]||at.z<domain_min[1]||at.z>domain_max[1]){result.reason=Reason::OutsideCoverage;return result;}
        let Some((entry,exit))=clip_xz(p,d,domain_min,domain_max,start,limit)else{
            return if self.coverage==Coverage::CompleteLoadedTerrain{finish(result,Status::Clear,Reason::None,limit,p,d,Vec3::ZERO)}else{result.reason=Reason::OutsideCoverage;result};};
        let mut stack=Vec::with_capacity(64);stack.push(Node{x:0,z:0,level:self.levels.len()-1,a:entry,b:exit});
        let budget=node_budget.min(MAX_NODE_BUDGET);let mut certified=entry;
        while let Some(node)=stack.pop(){
            if result.visited_nodes>=budget {return finish(result,Status::Unknown,Reason::Budget,node.a,p,d,Vec3::ZERO);}
            result.visited_nodes+=1;
            let level=&self.levels[node.level];let bounds=level.bounds[(node.z*level.width+node.x)as usize];
            let low_y=(p.y+d.y*node.a).min(p.y+d.y*node.b);
            // The complete affine ray interval lies above every triangle in
            // this node. Conservative rounding margin can only add work.
            let epsilon=1e-5+8.*f32::EPSILON as f64*low_y.abs().max(bounds[1].abs()as f64);
            if low_y>bounds[1]as f64+epsilon {certified=node.b;continue;}
            if node.level==0 {
                if let Some((status,reason,t,n))=self.leaf(p,d,node,&mut result.triangle_tests){return finish(result,status,reason,t,p,d,n);}
                certified=node.b;continue;
            }
            let mut children=Vec::with_capacity(4);let next=node.level-1;let l=&self.levels[next];let span=(1u32<<next)as f64*self.field.cell as f64;
            for dz in 0..2 {for dx in 0..2 {let x=node.x*2+dx;let z=node.z*2+dz;
                if x>=l.width||z>=l.height{continue;}
                let lo=[domain_min[0]+x as f64*span,domain_min[1]+z as f64*span];
                let hi=[(lo[0]+span).min(domain_max[0]),(lo[1]+span).min(domain_max[1])];
                if lo[0]>=hi[0]||lo[1]>=hi[1]{continue;}
                if let Some((a,b))=clip_xz(p,d,lo,hi,node.a,node.b){if b>a||node.a==node.b{children.push(Node{x,z,level:next,a,b});}}
            }}
            children.sort_by(|a,b|b.a.total_cmp(&a.a));
            if stack.len()+children.len()>64{return finish(result,Status::Unknown,Reason::Stack,node.a,p,d,Vec3::ZERO);}
            stack.extend(children);
        }
        // The field edge is not a declaration that the unrepresented world
        // is empty. Only a request ending inside coverage can return Clear.
        if exit+1e-8<limit && self.coverage==Coverage::UnknownOutside {finish(result,Status::Unknown,Reason::OutsideCoverage,certified,p,d,Vec3::ZERO)}
        else {finish(result,Status::Clear,Reason::None,limit,p,d,Vec3::ZERO)}
    }
    fn leaf(&self,p:DVec3,d:DVec3,node:Node,tests:&mut u32)->Option<(Status,Reason,f64,Vec3)> {
        let hf=&self.field;let cell=hf.cell as f64;let gx=(p.x-hf.origin[0]as f64)/cell-node.x as f64;let gz=(p.z-hf.origin[1]as f64)/cell-node.z as f64;
        let dx=d.x/cell;let dz=d.z/cell;let rate=dx+dz;
        let split=if rate.abs()>1e-30{(1.-gx-gz)/rate}else{f64::INFINITY};
        let middle=if split>node.a&&split<node.b{split}else{node.b};
        let k=(node.z*hf.w+node.x)as usize;let h=[hf.heights[k],hf.heights[k+1],hf.heights[k+hf.w as usize],hf.heights[k+hf.w as usize+1]];
        for (a,b) in [(node.a,middle),(middle,node.b)]{if b<a || (a==b&&node.a!=node.b){continue;}
            *tests+=1;let upper=gx+gz+rate*(a+b)*0.5>1.;
            let (u,v,w,values)=if upper{(h[1]+h[2]-h[3],h[3]-h[2],h[3]-h[1],[h[1],h[2],h[3]])}
                else{(h[0],h[1]-h[0],h[2]-h[0],[h[0],h[1],h[2]])};
            if triangle_empty(values,self.coverage){continue;}
            if !values.iter().copied().all(known){return Some((Status::Unknown,Reason::UnknownHeight,a,Vec3::ZERO));}
            let intercept=p.y-u as f64-v as f64*gx-w as f64*gz;let slope=d.y-v as f64*dx-w as f64*dz;
            let da=intercept+slope*a;let db=intercept+slope*b;
            if da<=0.||db<=0.{let t=if da<=0.{a}else{(-intercept/slope).clamp(a,b)};
                return Some((Status::Hit,Reason::None,t,Vec3::new(-v/hf.cell,1.,-w/hf.cell).normalize()));}
        }None
    }
    /// Certifies only the represented terrain. Other geometry must still be
    /// queried by the caller. Unknown source heights cannot certify a sky ray.
    pub fn trace_upward(&self,origin:Vec3,direction:Vec3,t_min:f32,node_budget:u32)->TraceResult {
        let maximum=self.levels.last().unwrap().bounds[0][1];
        let length2=direction.length_squared();
        if !origin.is_finite()||!direction.is_finite()||!length2.is_finite()||length2<=1e-20||direction.y<=0.||maximum>=1e29 {
            return TraceResult{status:Status::Unknown,reason:if maximum>=1e29{Reason::UnknownHeight}else{Reason::InvalidRay},distance:t_min,position:origin,normal:Vec3::ZERO,visited_nodes:0,triangle_tests:0};
        }
        let d=direction.normalize();
        let end=((maximum+0.01-origin.y)/d.y).max(t_min);
        let mut r=self.trace(Ray{origin,direction:d,t_min,t_max:end},node_budget);
        if r.status==Status::Clear{r.status=Status::AboveTerrainBounds;}r
    }
}
#[derive(Clone,Copy)]struct Node{x:u32,z:u32,level:usize,a:f64,b:f64}
fn finish(mut r:TraceResult,status:Status,reason:Reason,t:f64,p:DVec3,d:DVec3,n:Vec3)->TraceResult{
    r.status=status;r.reason=reason;r.distance=t as f32;r.position=(p+d*t).as_vec3();r.normal=n;r
}
fn clip_xz(p:DVec3,d:DVec3,lo:[f64;2],hi:[f64;2],mut a:f64,mut b:f64)->Option<(f64,f64)>{
    for (p,d,lo,hi) in [(p.x,d.x,lo[0],hi[0]),(p.z,d.z,lo[1],hi[1])]{
        if d.abs()<1e-30 {if p<lo||p>hi{return None;}}
        else {let x=(lo-p)/d;let y=(hi-p)/d;a=a.max(x.min(y));b=b.min(x.max(y));if b<a{return None;}}
    }Some((a,b))
}

#[path="terrain_trace_gpu.rs"]mod gpu;
pub use gpu::{GpuHierarchy,GpuBatch};

#[cfg(test)]mod tests{
    use super::*;
    fn hf(w:u32,h:u32,f:impl Fn(u32,u32)->f32)->Heightfield{Heightfield{origin:[0.,0.],cell:1.,w,h,heights:(0..h).flat_map(|z|(0..w).map(move|x|(x,z))).map(|(x,z)|f(x,z)).collect()}}
    fn ray(p:Vec3,d:Vec3,t:f32)->Ray{Ray{origin:p,direction:d,t_min:0.,t_max:t}}
    #[test]fn extreme_inputs_are_explicitly_invalid_not_different_cpu_gpu_rays(){
        let f=hf(2,2,|_,_|0.);let h=Hierarchy::build(&f).unwrap();
        for direction in [Vec3::splat(1e29),Vec3::splat(1e-30),Vec3::ZERO,Vec3::splat(f32::INFINITY)]{
            assert_eq!(h.trace(ray(Vec3::new(0.5,1.,0.5),direction,10.),256).reason,Reason::InvalidRay);
            assert_eq!(h.trace_upward(Vec3::new(0.5,1.,0.5),direction,0.,256).reason,Reason::InvalidRay);
        }
        let mut overflow=f;overflow.origin=[f32::MAX,0.];overflow.cell=f32::MAX;
        assert!(Hierarchy::build(&overflow).is_err());
    }
    #[test]fn empty_flat_space_skips_thousands_of_cells(){let h=Hierarchy::build(&hf(4097,3,|_,_|0.)).unwrap();let r=h.trace(ray(Vec3::new(0.1,1.,1.),Vec3::X,4000.),256);assert_eq!(r.status,Status::Clear);assert_eq!(r.visited_nodes,1);assert_eq!(r.triangle_tests,0);}
    #[test]fn ridge_beyond_previous_march_cutoff_is_hit(){let h=Hierarchy::build(&hf(2049,3,|x,_|if x==1500{40.}else{0.})).unwrap();let r=h.trace(ray(Vec3::new(1.,2.,1.),Vec3::new(1.,0.01,0.),1900.),256);assert_eq!(r.status,Status::Hit);assert!(r.distance>1400.&&r.distance<1500.);assert!(r.visited_nodes<64);assert!(r.normal.is_finite());}
    #[test]fn diagonal_is_actual_two_triangles_not_bilinear(){let h=Hierarchy::build(&hf(2,2,|x,z|if x!=z{10.}else{0.})).unwrap();let r=h.trace(ray(Vec3::new(0.1,6.,0.1),Vec3::new(1.,0.,1.),1.),256);assert_eq!(r.status,Status::Hit);assert!((r.distance-0.2*2f32.sqrt()).abs()<1e-5);}
    #[test]fn unknown_budget_and_world_exit_never_become_clear(){let mut f=hf(65,3,|x,_|if x==40{20.}else{0.});let h=Hierarchy::build(&f).unwrap();let r=ray(Vec3::new(1.,1.,1.),Vec3::X,60.);assert_eq!(h.trace(r,0).reason,Reason::Budget);assert_eq!(h.trace(ray(Vec3::new(1.,30.,1.),Vec3::X,100.),256).reason,Reason::OutsideCoverage);f.heights[20]=-1e9;let h=Hierarchy::build(&f).unwrap();let a=h.trace(ray(Vec3::new(1.,1.,0.25),Vec3::X,30.),256);assert_eq!(a.status,Status::Unknown);assert_eq!(a.reason,Reason::UnknownHeight);}
    #[test]fn slope_hit_and_world_translation_are_consistent(){let f=hf(65,65,|x,z|x as f32*0.2+z as f32*0.1);let h=Hierarchy::build(&f).unwrap();let p=Vec3::new(4.,30.,7.);let r=h.trace(ray(p,Vec3::NEG_Y,40.),256);assert_eq!(r.status,Status::Hit);assert!((r.distance-28.5).abs()<1e-5);assert!(r.normal.abs_diff_eq(Vec3::new(-0.2,1.,-0.1).normalize(),1e-5));let mut shifted=f.clone();shifted.origin=[8192.,-16384.];for v in &mut shifted.heights{*v+=128.;}let b=Hierarchy::build(&shifted).unwrap().trace(ray(p+Vec3::new(8192.,128.,-16384.),Vec3::NEG_Y,40.),256);assert!((b.distance-r.distance).abs()<2e-5);}
    #[test]fn coverage_policy_and_padding_are_explicit(){
        let mut f=hf(18,7,|_,_|4.);f.heights[0]=-1e9;
        let strict=Hierarchy::build(&f).unwrap();let complete=Hierarchy::with_coverage(&f,Coverage::CompleteLoadedTerrain).unwrap();
        assert_eq!(strict.trace_upward(Vec3::new(3.,10.,3.),Vec3::Y,0.,256).reason,Reason::UnknownHeight);
        assert_eq!(complete.trace_upward(Vec3::new(3.,10.,3.),Vec3::Y,0.,256).status,Status::AboveTerrainBounds);
        assert_eq!(complete.levels.last().unwrap().bounds[0][1],4.,"power-of-two padding cannot contaminate the real maximum");
        let outside=ray(Vec3::new(-10.,6.,3.),Vec3::X,100.);
        assert_eq!(strict.trace(outside,256).reason,Reason::OutsideCoverage);assert_eq!(complete.trace(outside,256).status,Status::Clear);
        f.heights[0]=f32::NAN;let malformed=Hierarchy::with_coverage(&f,Coverage::CompleteLoadedTerrain).unwrap();
        assert_eq!(malformed.trace_upward(Vec3::new(3.,10.,3.),Vec3::Y,0.,256).reason,Reason::UnknownHeight);
    }
    fn triangle_hit(p:DVec3,d:DVec3,a:DVec3,b:DVec3,c:DVec3)->Option<f64>{
        let e1=b-a;let e2=c-a;let cross=d.cross(e2);let determinant=e1.dot(cross);if determinant.abs()<1e-12{return None;}
        let q=p-a;let u=q.dot(cross)/determinant;if !(0.0..=1.0).contains(&u){return None;}
        let cross=q.cross(e1);let v=d.dot(cross)/determinant;if v<0.||u+v>1.{return None;}
        let t=e2.dot(cross)/determinant;if t>=0.{Some(t)}else{None}
    }
    #[test]fn hierarchical_first_hit_matches_independent_brute_triangle_oracle(){
        let f=hf(25,19,|x,z|{let x=x as f32;let z=z as f32;3.*(x*0.37).sin()+2.*(z*0.23).cos()+(x-12.).abs()*0.1});
        let h=Hierarchy::build(&f).unwrap();let mut seed=9u32;let mut random=||{seed=seed.wrapping_mul(1664525).wrapping_add(1013904223);(seed>>8)as f32/(1u32<<24)as f32};
        let mut max_nodes=0;let mut hits=0;
        for _ in 0..256{
            let p=Vec3::new(1.+22.*random(),15.,1.+16.*random());let d=Vec3::new(random()-0.5,-0.1-random(),random()-0.5).normalize();
            let r=h.trace(ray(p,d,40.),4096);max_nodes=max_nodes.max(r.visited_nodes);
            let mut closest=f64::INFINITY;
            for z in 0..f.h-1{for x in 0..f.w-1{
                let point=|x:u32,z:u32|DVec3::new(x as f64,f.heights[(z*f.w+x)as usize]as f64,z as f64);
                let a=point(x,z);let b=point(x+1,z);let c=point(x,z+1);let e=point(x+1,z+1);
                for (u,v,w) in [(a,b,c),(b,e,c)]{if let Some(t)=triangle_hit(p.as_dvec3(),d.as_dvec3().normalize(),u,v,w){closest=closest.min(t);}}
            }}
            if closest<=40.{hits+=1;assert_eq!(r.status,Status::Hit);assert!((r.distance as f64-closest).abs()<1e-4,"{} vs {closest}",r.distance);}
            else{assert_ne!(r.status,Status::Hit);}
        }
        assert!(hits>100);assert!(max_nodes<256);eprintln!("terrain hierarchy 256 independent rays: {hits} hits, max{max_nodes} nodes");
    }
}
