//! Finite swept road contacts, using the same ribbon subdivisions as the mesh.
//! Nearest-centreline projection is useful for progress, never collision authority.
use crate::{track::{Node,Track},handling::smooth};
use glam::Vec3;
#[derive(Clone,Copy,Debug)]
pub struct Hit {pub t:f32,pub distance:f64,pub normal:Vec3,pub top:bool}
pub fn road_span(a:&Node,b:&Node,side:f64,u:f64,v:f64)->[f64;4] {
    let bounds=|n:&Node| if side<0. {[-n.geometry.width,-n.split_gap]}else{[n.split_gap,n.geometry.width]};
    let x=bounds(a);let y=bounds(b);
    let lerp=|a:f64,b:f64,t:f64| a+(b-a)*t;
    [lerp(x[0],x[1],u),lerp(x[0],x[1],v),lerp(y[0],y[1],v),lerp(y[0],y[1],u)]
}
fn triangle(from:Vec3,delta:Vec3,a:Vec3,b:Vec3,c:Vec3)->Option<(f32,f32)> {
    let e=b-a;let f=c-a;let h=delta.cross(f);let det=e.dot(h);
    if det.abs()<1e-7 {return None;}
    let s=from-a;let u=s.dot(h)/det;let q=s.cross(e);let v=delta.dot(q)/det;
    let t=f.dot(q)/det;
    (u>=-0.0001 && v>=-0.0001 && u+v<=1.0001 && t>0.00001 && t<=1.).then_some((t,v))
}
impl Track {
    pub fn sweep_road(&self,from:Vec3,to:Vec3)->Option<Hit> {
        let delta=to-from;let mut best:Option<Hit>=None;
        for (i,a) in self.nodes.iter().enumerate() {
            if a.air_gap {continue;}
            let b=&self.nodes[(i+1)%self.nodes.len()];
            // Conservative segment broad phase also contains tubes and tunnel roofs.
            let reach=(a.geometry.width.max(b.geometry.width)*2.) as f32+35.;
            let lo=a.geometry.p.min(b.geometry.p)-Vec3::splat(reach);
            let hi=a.geometry.p.max(b.geometry.p)+Vec3::splat(reach);
            if from.min(to).cmpgt(hi).any() || from.max(to).cmplt(lo).any() {continue;}
            let mut quad=|points:[Vec3;4],normal:Vec3,top:bool,one_sided:bool| {
                if one_sided && delta.dot(normal)>=0. {return;}
                for ids in [[0,1,2],[0,2,3]] {
                    let [x,y,z]=ids.map(|j|points[j]);
                    if let Some((t,_))=triangle(from,delta,x,y,z) {
                        if best.is_some_and(|h|h.t<=t) {continue;}
                        let hit=from+delta*t;
                        let axis=b.geometry.p-a.geometry.p;
                        let fraction=((hit-a.geometry.p).dot(axis)/axis.length_squared().max(0.001)).clamp(0.,1.);
                        best=Some(Hit {t,distance:(i as f64+fraction as f64)*self.step,
                            normal:if !one_sided && delta.dot(normal)>0. {-normal}else{normal},top});
                    }
                }
            };
            for side in [-1.,1.] {
                if a.dead_gap && a.dead_side*side>0. {continue;}
                for lane in 0..8 {
                    let u=lane as f64/8.;let [x,y,z,w]=road_span(a,b,side,u,u+0.125);
                    let ns=[a.frame(x).y_axis,a.frame(y).y_axis,b.frame(z).y_axis,b.frame(w).y_axis];
                    let pts=[a.point(x,0.),a.point(y,0.),b.point(z,0.),b.point(w,0.)];
                    let normal=(ns[0]+ns[1]+ns[2]+ns[3]).normalize_or_zero();
                    quad(std::array::from_fn(|j|pts[j]+ns[j]*1.3),normal,true,true);
                    quad(std::array::from_fn(|j|pts[j]-ns[j]*0.7),-normal,false,true);
                }
                let mut edge=|x:f64,y:f64,ha:f64,hb:f64| {
                    quad([a.point(x,-2.5),a.point(x,ha),b.point(y,hb),b.point(y,-2.5)],a.frame(x).x_axis,false,false);
                };
                edge(side*a.geometry.width,side*b.geometry.width,
                    if a.rails && a.geometry.shape_angle<3. {2.4}else{0.},
                    if a.rails && b.geometry.shape_angle<3. {2.4}else{0.});
                if a.split_gap.max(b.split_gap)>0.0001 {
                    edge(side*a.split_gap,side*b.split_gap,2.4*smooth(0.,3.,a.split_gap),2.4*smooth(0.,3.,b.split_gap));
                }
            }
            if a.tunnel && a.geometry.shape_angle<0.1 {
                let point=|n:&Node,t:f32| n.geometry.p+n.geometry.basis()*Vec3::new(t.cos()*(n.geometry.width as f32+4.),t.sin()*30.,0.);
                for k in 0..12 {
                    let x=k as f32*std::f32::consts::PI/12.;let y=(k+1) as f32*std::f32::consts::PI/12.;
                    let pts=[point(a,x),point(a,y),point(b,y),point(b,x)];
                    let normal=(pts[1]-pts[0]).cross(pts[2]-pts[0]).normalize_or_zero();
                    quad(pts,normal,false,false);
                }
            }
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn flat()->Track {
        let mut t=crate::race::Race::new(35,"hard","cell",1,1).world.track;
        let mut n=t.nodes[0].clone();n.geometry.p=Vec3::ZERO;n.geometry.frame=glam::Mat3::IDENTITY.to_cols_array_2d();
        n.geometry.width=20.;n.geometry.shape_angle=0.;n.air_gap=false;n.dead_gap=false;n.split_gap=0.;n.rails=true;n.tunnel=false;
        n.before=None;n.after=None;
        t.nodes=(0..3).map(|i|{let mut n=n.clone();n.geometry.p.z=i as f32*100.;n}).collect();t.step=100.;t.length=300.;t
    }
    #[test]
    fn finite_road_catches_deck_underside_rail_and_tunnel() {
        let mut t=flat();
        assert!(t.sweep_road(Vec3::new(0.,20.,50.),Vec3::new(0.,-20.,50.)).unwrap().top);
        assert!(!t.sweep_road(Vec3::new(0.,-20.,50.),Vec3::new(0.,20.,50.)).unwrap().top);
        assert!(!t.sweep_road(Vec3::new(30.,1.,50.),Vec3::new(15.,1.,50.)).unwrap().top);
        assert!(t.sweep_road(Vec3::new(30.,-10.,50.),Vec3::new(15.,-10.,50.)).is_none());
        for n in &mut t.nodes {n.tunnel=true;}
        assert!(!t.sweep_road(Vec3::new(0.,20.,50.),Vec3::new(0.,40.,50.)).unwrap().top);
    }
    #[test]
    fn actual_gaps_are_empty_and_projection_switches_are_not_contacts() {
        let mut t=flat();for n in &mut t.nodes {n.air_gap=true;}
        assert!(t.sweep_road(Vec3::new(0.,20.,50.),Vec3::new(0.,-20.,50.)).is_none());
        for n in &mut t.nodes {n.air_gap=false;n.split_gap=8.;}
        assert!(t.sweep_road(Vec3::new(0.,20.,50.),Vec3::new(0.,-20.,50.)).is_none());
        assert!(t.sweep_road(Vec3::new(12.,20.,50.),Vec3::new(12.,-20.,50.)).is_some());
        // Flight beneath the deck and past a landing lip has no infinite wall.
        assert!(t.sweep_road(Vec3::new(0.,-30.,-10.),Vec3::new(0.,-30.,50.)).is_none());
    }
    #[test]
    fn feedback_spiral_46990_has_swept_sidewalls() {
        let t=crate::race::Race::new(46990,"hard","cell",1,1).world.track;
        let mut checks=0;
        for i in 0..t.nodes.len() {
            let a=&t.nodes[i];let b=&t.nodes[(i+1)%t.nodes.len()];
            if a.air_gap || !a.rails || a.dead_gap || a.geometry.shape_angle>0.1 {continue;}
            let edge=(a.point(a.geometry.width,1.)+b.point(b.geometry.width,1.))*0.5;
            let side=a.frame(a.geometry.width).x_axis;
            assert!(t.sweep_road(edge-side*3.,edge+side*3.).is_some(),"segment {i}");checks+=1;
        }
        assert!(checks>30);
    }
}
