//! Conservative terrain-tile support for procedural meadow shadow receivers.
//! Source roots stay GPU-generated. These immutable coarse terrain bounds only
//! request shadow work; they never remove plants or change their lighting.
use glam::{Mat4, Vec3};
use crate::{debug_settings::DebugRenderOptions, meadow_data::MeadowField,
    receiver_shadow_demand::ReceiverBounds};

struct Tile { min: Vec3, max: Vec3, slope: f32 }
pub(crate) struct MeadowReceivers {
    tiles: Vec<Tile>, growth: f32, anchor: f32, blade: f32, angle: f32,
    ranges: [f32; 2], aggregate: bool,
}
impl MeadowReceivers {
    pub fn new(field: &MeadowField, aggregate: bool) -> Result<Self, String> {
        field.validate()?;
        let mut tiles=Vec::new(); let w=field.width as usize; let h=field.height as usize;
        // Eight native cells per tile bounds request upload independently of
        // plant count, while retaining local heights on mountainous stages.
        for z in (0..h-1).step_by(8) { for x in (0..w-1).step_by(8) {
            let xe=(x+8).min(w-1);let ze=(z+8).min(h-1);
            let mut low=f32::INFINITY;let mut high=f32::NEG_INFINITY;let mut slope=0f32;
            for j in z..ze { for i in x..xe {
                let a=field.samples[j*w+i];let b=field.samples[j*w+i+1];
                let c=field.samples[(j+1)*w+i];let d=field.samples[(j+1)*w+i+1];
                for tri in [[a,b,c],[d,c,b]] {
                    if tri.iter().any(|v|v[2]<0.5) {continue;}
                    let heights=tri.map(|v|v[0]);
                    if heights.iter().any(|v|!v.is_finite()) {return Err("invalid meadow receiver heights".into());}
                    low=low.min(heights.into_iter().fold(f32::INFINITY,f32::min));
                    high=high.max(heights.into_iter().fold(f32::NEG_INFINITY,f32::max));
                    slope=slope.max(((heights[1]-heights[0]).powi(2)+(heights[2]-heights[0]).powi(2)).sqrt()/field.cell);
                }
            }}
            if low>high {continue;}
            if !slope.is_finite() {return Err("invalid meadow receiver slope".into());}
            tiles.push(Tile {min:Vec3::new(field.origin[0]+x as f32*field.cell,low-0.005,field.origin[1]+z as f32*field.cell),
                max:Vec3::new(field.origin[0]+xe as f32*field.cell,high-0.005,field.origin[1]+ze as f32*field.cell),slope});
        }}
        Ok(Self {tiles,growth:field.growth,anchor:field.geometry.max_anchor_radius,
            blade:field.geometry.max_blade_radius,
            angle:field.geometry.branch_pivots.iter().flatten().map(|p|p[3].abs()).fold(0.0,f32::max),
            ranges:[field.fade_start,field.radius],aggregate})
    }
    pub fn append(&self,camera:Vec3,vp:Mat4,focal_pixels:f32,map_mask:u32,options:DebugRenderOptions,out:&mut Vec<ReceiverBounds>) {
        if !options.draws_meadow() {return;}
        let ranges=crate::debug_settings::meadow_ranges(self.ranges,options.meadow_distance_scale);
        let range=if self.aggregate {ranges[3]} else {ranges[1]};
        let growth=self.growth*options.meadow_size_scale*1.3;
        let base=self.anchor+self.blade*growth;
        for tile in &self.tiles {
            // Vertical shear norm <=1+|slope|. All branch rotations preserve
            // anchor-relative lengths, independent of wind phase/strength.
            let shear=1.0+tile.slope;
            let mut support=base*shear+0.01;
            let dx=(tile.min.x-camera.x).max(0.0).max(camera.x-tile.max.x);
            let dz=(tile.min.z-camera.z).max(0.0).max(camera.z-tile.max.z);
            if dx*dx+dz*dz>(range+support).powi(2) {continue;}
            let far_x=(tile.min.x-camera.x).abs().max((tile.max.x-camera.x).abs());
            let far_z=(tile.min.z-camera.z).abs().max((tile.max.z-camera.z).abs());
            // The shared GPU producer rejects aggregate roots strictly below
            // fade-start and at/after range. Thus nearby tiles containing only
            // source geometry do not trigger an unnecessary camera-plane fallback.
            if self.aggregate && far_x.hypot(far_z)>=ranges[0] && dx.hypot(dz)<ranges[3] {
                let lower=tile.min.as_dvec3();let upper=tile.max.as_dvec3();let eye=camera.as_dvec3();
                let closest=(lower-eye).max(glam::DVec3::ZERO).max(eye-upper).length();
                let farthest=(lower-eye).abs().max((upper-eye).abs()).length();
                let min_distance=closest.max(ranges[0] as f64);
                // Atlas points/anchors are binary16 conditional means of
                // source vertices. Include their rounding before growth and
                // rigid branch rotation; independent means cannot cancel it.
                let quantize=|radius:f32|3.0f64.sqrt()*(radius as f64/2048.0+2.0f64.powi(-25));
                let ea=quantize(self.anchor);let ep=quantize(self.anchor+self.blade);
                let representative=(self.anchor as f64+ea+growth as f64*(self.blade as f64+ea+ep))*shear as f64;
                // meadow_aggregate_vs packs independent anchor/relative AABBs.
                // Each coordinate is bounded by A+growth*B+wind_margin; the
                // AABB's corner norm is at most sqrt(3) times this quantity.
                let margin=self.blade as f64*growth as f64*self.angle as f64*1.5;
                let corners=3.0f64.sqrt()*(base as f64+margin)*shear as f64;
                let Some(bound)=aggregate_ray_support(representative,corners,min_distance,farthest,focal_pixels as f64) else {
                    request_full_demand(map_mask,out);return;
                };
                // Include f32 world/projection arithmetic. This is a spatial
                // request expansion, never a lighting/depth bias.
                let roundoff=(eye.abs().max_element()+lower.abs().max(upper.abs()).max_element()+bound+1.0)*3.814697265625e-6;
                support=support.max((bound+roundoff+0.01) as f32);
                if !support.is_finite() {request_full_demand(map_mask,out);return;}
            }
            let min=tile.min-Vec3::splat(support);let max=tile.max+Vec3::splat(support);
            if !intersects_frustum(min,max,vp) {continue;}
            out.push(ReceiverBounds {min,max,map_mask});
        }
    }
}
// Bound the actual shading point, not only a source vertex. If p is within R
// of its root and the pixel ray contains billboard point q within B, then
// |Proj_ray(p)-root| <= R+B (orthogonal projection is nonexpansive).
fn aggregate_ray_support(source_radius:f64,corner_radius:f64,near:f64,far:f64,focal:f64)->Option<f64> {
    if ![source_radius,corner_radius,near,far,focal].into_iter().all(f64::is_finite)
        || source_radius<0.0 || corner_radius<0.0 || far<near || near<=corner_radius || focal<=0.0 {return None;}
    // Exact VS perspective ratio d/max(d-dot(corner,toward),1e-4).
    // For d>Q, Q*d/(d-Q) decreases with distance; one-pixel padding
    // d/max(focal,1) increases. Their independent extrema bound every root in
    // the tile. The projected rectangle has two independently bounded axes.
    let projected=corner_radius*near/(near-corner_radius);
    let pixel_pad=far/focal.max(1.0);
    Some(source_radius+2.0f64.sqrt()*(projected+pixel_pad))
}
fn request_full_demand(map_mask:u32,out:&mut Vec<ReceiverBounds>) {
    // Explicit fail-open sentinel consumed on the CPU by receiver demand's
    // INVALID_BOUNDS path. It is not uploaded as geometry or a NaN to the GPU.
    out.push(ReceiverBounds {min:Vec3::splat(-1e18),max:Vec3::splat(1e18),map_mask});
}
fn intersects_frustum(min:Vec3,max:Vec3,vp:Mat4)->bool {
    let center=(min+max)*0.5;let extent=(max-min)*0.5;
    let t=vp.transpose();let rows=[t.x_axis,t.y_axis,t.z_axis,t.w_axis];
    // WGPU Z in[0,W], including infinite reversed-Z. Degenerate far plane
    // never rejects. Numerical uncertainty over-requests rather than culls.
    [rows[3]+rows[0],rows[3]-rows[0],rows[3]+rows[1],rows[3]-rows[1],rows[2],rows[3]-rows[2]].iter().all(|p| {
        let d=p.truncate().dot(center)+p.w+p.truncate().abs().dot(extent);
        !d.is_finite() || d>=-1e-3*p.truncate().length().max(1.0)
    })
}
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn frustum_keeps_support_crossing_edge_and_rejects_far_outside() {
        let vp=Mat4::perspective_infinite_reverse_rh(1.0,1.0,0.1);
        assert!(intersects_frustum(Vec3::new(-1.,-1.,-4.),Vec3::new(1.,1.,-2.),vp));
        assert!(intersects_frustum(Vec3::new(-3.,-1.,-4.),Vec3::new(-1.,1.,-2.),vp));
        assert!(!intersects_frustum(Vec3::new(20.,-1.,-4.),Vec3::new(21.,1.,-2.),vp));
        assert!(intersects_frustum(Vec3::splat(-0.2),Vec3::splat(0.2),vp));
    }
    #[test] fn shear_rotation_bound_covers_anchors_and_blades() {
        for slope in [0.,1.,4.,20.] {for a in 0..64 {
            let angle=a as f32*0.1;let local=Vec3::new(angle.cos(),angle.sin(),0.)*0.7+Vec3::X*0.3;
            let actual=local+Vec3::Y*(slope*local.x);
            assert!(actual.length()<=1.0*(1.0+slope)+1e-5);
        }}
    }
    #[test] fn aggregate_projection_bound_covers_billboard_corners_and_off_ray_points() {
        use glam::DVec3;
        let mut outside_source_support=0;
        // Independent f64 reproduction of VS projection and fragment ray
        // reconstruction, including rectangle corners not on source geometry.
        for slope in [0.0f64,0.8,4.0] {for growth in [0.35f64,1.3,3.9] {for distance in [20.0f64,80.0,180.0] {
            let source_radius=(0.38+0.5*growth)*(1.0+slope);
            let extent=0.38+0.5*growth+0.5*growth*0.22*1.5;
            let corner_radius=3.0f64.sqrt()*extent*(1.0+slope);
            let Some(bound)=aggregate_ray_support(source_radius,corner_radius,distance,distance,300.0) else {continue;};
            for pitch in [-0.7f64,0.0,0.6,1.56] {for yaw in [0.0f64,0.4,2.5] {
                let toward=DVec3::new(pitch.cos()*yaw.cos(),pitch.sin(),pitch.cos()*yaw.sin());
                let eye=toward*distance;
                let right=if toward.y.abs()>0.999 {DVec3::X}else{DVec3::Y.cross(toward).normalize()};
                let up=toward.cross(right);let mut low=glam::DVec2::splat(f64::INFINITY);let mut high=-low;
                for k in 0..8 {
                    let p=DVec3::new(if k&1==0{-extent}else{extent},if k&2==0{-extent}else{extent},if k&4==0{-extent}else{extent});
                    let p=p+DVec3::Y*(slope*p.x);
                    let ratio=distance/(distance-p.dot(toward)).max(1e-4);
                    let projected=glam::DVec2::new(p.dot(right),p.dot(up))*ratio;
                    low=low.min(projected);high=high.max(projected);
                }
                low-=glam::DVec2::splat(distance/300.0);high+=glam::DVec2::splat(distance/300.0);
                for ix in 0..5 {for iy in 0..5 {
                    let xy=low.lerp(high,ix as f64/4.0);let xy=glam::DVec2::new(xy.x,low.y+(high.y-low.y)*iy as f64/4.0);
                    let q=right*xy.x+up*xy.y;let ray=(q-eye).normalize();
                    for p in [DVec3::X,DVec3::Y,DVec3::Z,-DVec3::X,-DVec3::Y,-DVec3::Z] {
                        let p=p*source_radius;let reconstructed=eye+ray*(p-eye).dot(ray);
                        assert!(reconstructed.length()<=bound+1e-10,"{slope} {growth} {distance} {pitch} {yaw}");
                        outside_source_support+=usize::from(reconstructed.length()>source_radius+1e-6);
                    }
                }}
            }}
        }}}
        assert!(outside_source_support>100,"must exercise actual reconstructed points outside physical source support");
    }
    #[test] fn aggregate_uncertainty_requests_explicit_full_demand() {
        for (near,far,focal) in [(1.0,2.0,300.0),(2.0,1.0,300.0),(4.0,6.0,0.0),(f64::NAN,6.0,300.0)] {
            assert!(aggregate_ray_support(1.0,2.0,near,far,focal).is_none());
        }
        let mut bounds=Vec::new();request_full_demand(3,&mut bounds);
        assert_eq!(bounds[0].map_mask,3);
        assert!(bounds[0].min.is_finite() && bounds[0].max.is_finite());
        assert!(bounds[0].min.abs().max_element()>=1e18); // provider INVALID_BOUNDS -> all tiles
        // Distance-dependent one-pixel padding must not disappear at high range.
        assert!(aggregate_ray_support(1.0,2.0,20.0,180.0,300.0).unwrap()
            >aggregate_ray_support(1.0,2.0,20.0,20.0,300.0).unwrap());
    }
}
