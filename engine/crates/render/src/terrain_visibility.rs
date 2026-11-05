//! Terrain-only visibility of the solar disc from one camera.
//!
//! The horizontal ray walks every crossed heightfield cell, then splits at
//! the same x+z=1 diagonal as `ground_at`. Height is affine on each segment,
//! so (height-camera_y)/distance is monotonic and its extrema are endpoints.
//! No fixed march step can skip a narrow ridge, and no allocation is needed.
//! The finite-disc result assumes the sampled horizon is locally constant
//! across the sun's azimuthal width; it is not a full 2D disc/terrain integral.

use crate::scene::Heightfield;
use glam::Vec3;

const SUN_RADIUS: f64 = 0.00465;
const UNKNOWN_HEIGHT: f64 = -1.0e8;

/// Direct solar visibility, independent for each camera and independent of
/// its look direction. Unknown terrain outside the field (or invalid source
/// triangles) does not block. A below-horizon/invalid sun contributes nothing.
pub(crate) fn camera_sun_visibility(hf: &Heightfield, camera: Vec3, sun_direction: Vec3) -> f32 {
    if !sun_direction.is_finite() || sun_direction.y <= 0.0 {
        return 0.0;
    }
    if !camera.is_finite() || !valid_field(hf) {
        return 1.0;
    }
    let sx=sun_direction.x as f64;let sy=sun_direction.y as f64;let sz=sun_direction.z as f64;
    let horizontal=sx.hypot(sz);
    let cx=camera.x as f64;let cy=camera.y as f64;let cz=camera.z as f64;
    let cell=hf.cell as f64;let ox=hf.origin[0] as f64;let oz=hf.origin[1] as f64;
    let width=(hf.w-1) as f64;let height=(hf.h-1) as f64;
    let gx=(cx-ox)/cell;let gz=(cz-oz)/cell;
    // Includes a camera exactly on the field border pointing outwards: it
    // is still underground even though the traversed horizontal slab is empty.
    if point_height(hf,gx,gz).is_some_and(|h|h>cy) {return 0.0;}
    // An exactly upward ray never leaves its column. This also avoids
    // dividing by a vanishing horizontal direction at the zenith.
    if horizontal <= sy.abs()*1.0e-12 {
        return 1.0;
    }
    let dx=sx/horizontal;let dz=sz/horizontal;
    let sun_angle=sy.atan2(horizontal);
    let mut entry=0.0f64;let mut exit=f64::INFINITY;
    if !clip_slab(cx,dx,ox,ox+width*cell,&mut entry,&mut exit)
        || !clip_slab(cz,dz,oz,oz+height*cell,&mut entry,&mut exit)
        || exit<=entry {
        return 1.0;
    }
    let ix_start=(gx+dx*entry/cell).clamp(0.0,width);
    let iz_start=(gz+dz*entry/cell).clamp(0.0,height);
    let mut ix=forward_cell(ix_start,dx,hf.w-1);
    let mut iz=forward_cell(iz_start,dz,hf.h-1);
    let step_x=if dx<0.0{-1}else{1};let step_z=if dz<0.0{-1}else{1};
    let delta_x=if dx==0.0{f64::INFINITY}else{cell/dx.abs()};
    let delta_z=if dz==0.0{f64::INFINITY}else{cell/dz.abs()};
    let next_x=|ix:i32|if dx==0.0{f64::INFINITY}else{
        (ox+(ix+if dx>0.0{1}else{0}) as f64*cell-cx)/dx};
    let next_z=|iz:i32|if dz==0.0{f64::INFINITY}else{
        (oz+(iz+if dz>0.0{1}else{0}) as f64*cell-cz)/dz};
    let mut tx=next_x(ix).max(entry);let mut tz=next_z(iz).max(entry);
    let mut t=entry;let mut horizon=f64::NEG_INFINITY;
    // Only a known horizon above the *whole* solar disc can end the query.
    let hidden_slope=if sun_angle+SUN_RADIUS<std::f64::consts::FRAC_PI_2 {
        (sun_angle+SUN_RADIUS).tan()
    }else{f64::INFINITY};
    // A line crosses at most (w-1)+(h-1) cells. The guard also makes numeric
    // failure bounded without allocating a visited set.
    for _ in 0..(hf.w as u64+hf.h as u64) {
        if ix<0 || iz<0 || ix>=hf.w as i32-1 || iz>=hf.h as i32-1 {break;}
        let end=tx.min(tz).min(exit);
        if end>t {
            cell_horizon(hf,ix as usize,iz as usize,gx,gz,dx/cell,dz/cell,cy,t,end,&mut horizon);
            if horizon>=hidden_slope {return 0.0;}
        }
        if end>=exit {break;}
        // Advance both axes at a corner; no tiny cell or diagonal can be
        // skipped. A tie need not be exact: an ulp-sized interval is safe.
        if tx<=end {ix+=step_x;tx+=delta_x;}
        if tz<=end {iz+=step_z;tz+=delta_z;}
        t=end;
    }
    if horizon==f64::INFINITY {return 0.0;}
    disc_visibility(sun_angle-horizon.atan())
}

fn valid_field(hf:&Heightfield)->bool {
    hf.w>=2 && hf.h>=2 && hf.w<=i32::MAX as u32 && hf.h<=i32::MAX as u32
        && hf.cell.is_finite() && hf.cell>0.0 && hf.origin.iter().all(|x|x.is_finite())
        && (hf.w as usize).checked_mul(hf.h as usize).is_some_and(|n|n<=hf.heights.len())
}

fn clip_slab(p:f64,d:f64,lo:f64,hi:f64,entry:&mut f64,exit:&mut f64)->bool {
    if d==0.0 {return p>=lo && p<=hi;}
    let a=(lo-p)/d;let b=(hi-p)/d;
    *entry=entry.max(a.min(b));*exit=exit.min(a.max(b));
    *exit>=*entry
}

fn forward_cell(g:f64,d:f64,count:u32)->i32 {
    let mut index=g.floor() as i32;
    if d<0.0 && g==g.floor() {index-=1;}
    index.clamp(0,count as i32-1)
}

fn valid_height(h:f64)->bool {h.is_finite() && h>UNKNOWN_HEIGHT}

/// Plane h = a + bx*f.x + bz*f.z in one native cell.
fn triangle_plane(hf:&Heightfield,ix:usize,iz:usize,upper:bool)->Option<[f64;3]> {
    let k=iz*hf.w as usize+ix;
    let a=hf.heights[k] as f64;let b=hf.heights[k+1] as f64;
    let c=hf.heights[k+hf.w as usize] as f64;let e=hf.heights[k+hf.w as usize+1] as f64;
    if upper {
        if [b,c,e].into_iter().all(valid_height) {Some([b+c-e,e-c,e-b])}else{None}
    }else if [a,b,c].into_iter().all(valid_height) {Some([a,b-a,c-a])}else{None}
}

fn point_height(hf:&Heightfield,gx:f64,gz:f64)->Option<f64> {
    if gx<0.0 || gz<0.0 || gx>(hf.w-1) as f64 || gz>(hf.h-1) as f64{return None;}
    let ix=(gx.floor() as usize).min(hf.w as usize-2);
    let iz=(gz.floor() as usize).min(hf.h as usize-2);
    let x=gx-ix as f64;let z=gz-iz as f64;
    let [a,b,c]=triangle_plane(hf,ix,iz,x+z>1.0)?;
    Some(a+b*x+c*z)
}

#[allow(clippy::too_many_arguments)]
fn cell_horizon(hf:&Heightfield,ix:usize,iz:usize,gx:f64,gz:f64,dx:f64,dz:f64,
    camera_y:f64,start:f64,end:f64,horizon:&mut f64) {
    let x=gx-ix as f64;let z=gz-iz as f64;
    let rate=dx+dz;
    let diagonal=if rate!=0.0{(1.0-x-z)/rate}else{f64::INFINITY};
    let split=diagonal>start && diagonal<end;
    let middle=if split{diagonal}else{end};
    for (a,b) in [(start,middle),(middle,end)] {
        if b<=a {continue;}
        let upper=x+z+rate*(0.5*(a+b))>1.0;
        let Some([p,q,r])=triangle_plane(hf,ix,iz,upper)else{continue;};
        let intercept=p+q*x+r*z-camera_y;let gradient=q*dx+r*dz;
        for t in [a,b] {
            let slope=if t>0.0 {intercept/t+gradient}
                else if intercept>0.0 {f64::INFINITY}
                else if intercept<0.0 {f64::NEG_INFINITY}
                else {gradient};
            *horizon=horizon.max(slope);
        }
    }
}

/// Area of a unit disc above a straight horizon. Exact circular-segment
/// integral, instead of a linear brightness ramp or camera-random sampling.
fn disc_visibility(angle_above_horizon:f64)->f32 {
    let x=(angle_above_horizon/SUN_RADIUS).clamp(-1.0,1.0);
    (0.5+(x.asin()+x*(1.0-x*x).max(0.0).sqrt())/std::f64::consts::PI) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    fn field(w:u32,h:u32,cell:f32,heights:Vec<f32>)->Heightfield {
        Heightfield{origin:[0.0,0.0],cell,w,h,heights}
    }
    fn sun(elevation:f32)->Vec3 {Vec3::new(elevation.cos(),elevation.sin(),0.0)}

    #[test]
    fn narrow_ridge_between_large_march_steps_blocks_sun() {
        let mut heights=vec![0.0;202];heights[37]=12.0;heights[138]=12.0;
        let hf=field(101,2,1.0,heights);
        // A 10 m marcher only sees y=0 at x30 and x40, but this 2 m ridge
        // rises 11 m above the eye and blocks the complete 10-degree disc.
        assert_eq!(camera_sun_visibility(&hf,Vec3::new(0.0,1.0,0.5),sun(10f32.to_radians())),0.0);
        assert_eq!(camera_sun_visibility(&hf,Vec3::new(0.0,1.0,0.5),sun(25f32.to_radians())),1.0);
    }

    #[test]
    fn native_triangle_diagonal_is_not_bilinear_or_cell_endpoints() {
        // Along x=z, the diagonal x+z=1 reaches y=10 halfway through the
        // cell; both cell endpoints are y=0. A bilinear patch peaks at y=5.
        let hf=field(2,2,10.0,vec![0.0,10.0,10.0,0.0]);
        let camera=Vec3::new(-10.0,1.0,-10.0);
        let elevation=(9.0f32/(15.0*2f32.sqrt())).atan();
        let direction=Vec3::new(elevation.cos()/2f32.sqrt(),elevation.sin(),elevation.cos()/2f32.sqrt());
        let v=camera_sun_visibility(&hf,camera,direction);
        assert!((v-0.5).abs()<2e-5,"the actual diagonal ridge bisects the sun: {v}");
    }

    #[test]
    fn outside_origins_negative_traversal_and_world_translation_agree() {
        let mut hf=field(3,3,2.0,vec![0.0,4.0,0.0,0.0,4.0,0.0,0.0,4.0,0.0]);
        let left=Vec3::new(-4.0,1.0,2.0);let right=Vec3::new(8.0,1.0,2.0);
        let d=sun((3.0f32/6.0).atan());
        let a=camera_sun_visibility(&hf,left,d);let b=camera_sun_visibility(&hf,right,Vec3::new(-d.x,d.y,0.0));
        assert!((a-0.5).abs()<2e-5 && (a-b).abs()<2e-5);
        hf.origin=[8192.0,-16384.0];for h in &mut hf.heights{*h+=128.0;}
        let translated=camera_sun_visibility(&hf,left+Vec3::new(8192.0,128.0,-16384.0),d);
        assert_eq!(a.to_bits(),translated.to_bits());
        assert_eq!(camera_sun_visibility(&hf,Vec3::ZERO,sun(0.1)),1.0,"outside ray misses field");
    }

    #[test]
    fn below_ground_vertical_and_below_horizon_are_finite() {
        let hf=field(3,3,2.0,vec![5.0;9]);
        assert_eq!(camera_sun_visibility(&hf,Vec3::new(1.0,4.0,1.0),sun(0.3)),0.0);
        assert_eq!(camera_sun_visibility(&hf,Vec3::new(1.0,4.0,1.0),Vec3::Y),0.0);
        assert_eq!(camera_sun_visibility(&hf,Vec3::new(0.0,4.0,1.0),Vec3::new(-1.0,0.3,0.0)),0.0);
        assert_eq!(camera_sun_visibility(&hf,Vec3::new(1.0,5.0,1.0),Vec3::Y),1.0);
        assert_eq!(camera_sun_visibility(&hf,Vec3::new(1.0,6.0,1.0),Vec3::Y),1.0);
        assert_eq!(camera_sun_visibility(&hf,Vec3::new(1.0,6.0,1.0),sun(-0.01)),0.0);
        assert_eq!(camera_sun_visibility(&hf,Vec3::new(1.0,6.0,1.0),Vec3::ZERO),0.0);
    }

    #[test]
    fn analytic_partial_disc_and_independent_view_origins() {
        let hf=field(3,2,10.0,vec![0.0,4.0,0.0,0.0,4.0,0.0]);
        let camera=Vec3::new(0.0,1.0,5.0);let horizon=(3.0f64/10.0).atan();
        let half=camera_sun_visibility(&hf,camera,sun(horizon as f32));
        let quarter_offset=camera_sun_visibility(&hf,camera,sun((horizon+SUN_RADIUS*0.5) as f32));
        // Independent analytic disc area at x=1/2: 2/3+sqrt(3)/(4*pi).
        let expected=(2.0/3.0+3f64.sqrt()/(4.0*std::f64::consts::PI)) as f32;
        assert!((half-0.5).abs()<1e-5);assert!((quarter_offset-expected).abs()<1e-5);
        assert_eq!(camera_sun_visibility(&hf,camera,sun((horizon+SUN_RADIUS*1.1) as f32)),1.0);
        assert_eq!(camera_sun_visibility(&hf,camera,sun((horizon-SUN_RADIUS*1.1) as f32)),0.0);
        let low=camera_sun_visibility(&hf,Vec3::new(0.0,0.5,5.0),sun(horizon as f32));
        let high=camera_sun_visibility(&hf,Vec3::new(0.0,4.5,5.0),sun(horizon as f32));
        assert_eq!((low,high),(0.0,1.0));
    }

    #[test]
    fn unknown_and_invalid_samples_do_not_create_a_horizon_wall() {
        for empty in [-1e9,f32::NAN,f32::INFINITY] {
            let hf=field(3,2,2.0,vec![empty;6]);
            assert_eq!(camera_sun_visibility(&hf,Vec3::new(-1.0,1.0,1.0),sun(0.05)),1.0);
        }
        let malformed=field(100,100,2.0,vec![]);
        assert_eq!(camera_sun_visibility(&malformed,Vec3::ZERO,sun(0.1)),1.0);
        // Invalid fourth corner must not erase the other, known triangle.
        let partial=field(2,2,2.0,vec![4.0,4.0,4.0,-1e9]);
        assert_eq!(camera_sun_visibility(&partial,Vec3::new(0.25,1.0,0.25),sun(0.2)),0.0);
    }

    #[test]
    fn four_view_query_cost_is_bounded_by_grid_crossings() {
        let hf=field(1025,1025,2.0,vec![0.0;1025*1025]);
        let cameras=[Vec3::new(-1.0,2.0,10.0),Vec3::new(100.0,2.0,20.0),
            Vec3::new(700.0,2.0,1000.0),Vec3::new(2040.0,2.0,2040.0)];
        let directions=[Vec3::new(1.0,0.1,0.03),Vec3::new(1.0,0.1,0.7),
            Vec3::new(-1.0,0.1,-0.3),Vec3::new(-1.0,0.1,-1.0)];
        let begin=web_time::Instant::now();let mut sum=0.0;
        for _ in 0..128 {for i in 0..4 {
            sum+=std::hint::black_box(camera_sun_visibility(std::hint::black_box(&hf),cameras[i],directions[i]));
        }}
        assert_eq!(sum,512.0);
        eprintln!("terrain horizon 1025x1025 / 2m: {:.3} us per four independent views (128 batches, flat unblocked worst-length rays, CPU only)",begin.elapsed().as_secs_f64()*1e6/128.0);
    }
}
