//! Independent mathematical regression oracles for the rock material's
//! world-fixed texture mapping. Production executes the WGSL; these tests
//! compare continuity and gradients to finite differences of the UV field.
use glam::{Mat2, Vec2, Vec3};

fn hash(cell: [i32; 2], seed: u32) -> f32 {
    let mut h = (cell[0] as u32).wrapping_mul(0x9e3779b9)
        ^ (cell[1] as u32).wrapping_mul(0x85ebca6b) ^ seed;
    h ^= h >> 16; h = h.wrapping_mul(0x7feb352d); h ^= h >> 15;
    h = h.wrapping_mul(0x846ca68b); h ^= h >> 16;
    (h >> 8) as f32 / 16_777_216.0
}
fn noise(p: Vec2, seed: u32) -> Vec3 {
    let cell = p.floor().as_ivec2(); let f = p - p.floor();
    let u = f*f*f*(f*(f*6.0-Vec2::splat(15.0))+Vec2::splat(10.0));
    let du = 30.0*f*f*(f-Vec2::ONE)*(f-Vec2::ONE);
    let a=hash([cell.x,cell.y],seed); let b=hash([cell.x+1,cell.y],seed);
    let c=hash([cell.x,cell.y+1],seed); let d=hash([cell.x+1,cell.y+1],seed);
    Vec3::new((a+(b-a)*u.x+(c+(d-c)*u.x-a-(b-a)*u.x)*u.y)*2.0-1.0,
        ((b-a)+(d-c-b+a)*u.y)*du.x*2.0,
        ((c-a)+(d-b-c+a)*u.x)*du.y*2.0)
}
fn warp(p:Vec2,seed:u32)->Vec3 {
    let a=noise(p/13.0,seed);let b=noise(p/47.0,seed^0x68bc21eb);
    Vec3::new(a.x*0.8+b.x*2.6,a.y*(0.8/13.0)+b.y*(2.6/47.0),a.z*(0.8/13.0)+b.z*(2.6/47.0))
}
fn variant(index:i32)->(Mat2,Vec2) {
    let angle=hash([index,37],0xc28491d3)*std::f32::consts::TAU;
    (Mat2::from_angle(angle),Vec2::new(hash([index,11],0x6a09e667),hash([index,23],0xbb67ae85)))
}
fn blend(selector:f32)->(i32,f32) {
    let i=selector.floor();let t=((selector-i-0.15)/0.7).clamp(0.0,1.0);
    (i as i32,t*t*(3.0-2.0*t))
}
fn pattern(q:Vec2)->Vec3 {
    // A synthetic, smooth periodic texture exposes UV seams independently
    // of any visual asset, lighting, camera, or tonemapping.
    Vec3::new((q.x*6.2831853).sin(),(q.y*6.2831853).cos(),((q.x+q.y)*6.2831853).sin())
}
fn mapped(selector:f32,p:Vec2)->Vec3 {
    let(i,t)=blend(selector);let(a,ao)=variant(i);let(b,bo)=variant(i+1);
    pattern(a*p+ao).lerp(pattern(b*p+bo),t)
}

#[test]
fn variant_boundaries_share_the_same_source_and_scale() {
    for i in -24..24 { for p in [Vec2::new(-17.0,31.0),Vec2::new(0.23,-0.17)] {
        assert!((mapped(i as f32-0.0001,p)-mapped(i as f32+0.0001,p)).length()<1e-6);
        let(r,_)=variant(i);
        assert!((r.determinant()-1.0).abs()<1e-6);
        assert!(((r*Vec2::new(0.6,0.8)).length()-1.0).abs()<1e-6);
        assert!((r.x_axis.dot(r.y_axis)).abs()<1e-6);
    }}
}

#[test]
fn noise_cell_edges_and_warp_jacobian_remain_continuous() {
    for point in [Vec2::new(-13.0,3.4),Vec2::new(0.0,-7.8),Vec2::new(47.0,-13.0)] {
        for axis in [Vec2::X,Vec2::Y] {
            let a=warp(point-axis*0.0001,0x7314a5d3);let b=warp(point+axis*0.0001,0x7314a5d3);
            assert!((a-b).length()<0.0002);
        }
    }
    for point in [Vec2::new(-123.4,77.8),Vec2::new(0.2,-0.1),Vec2::new(301.4,-423.7)] {
        let analytic=warp(point,0x7314a5d3);
        for(axis,derivative)in[(Vec2::X,analytic.y),(Vec2::Y,analytic.z)] {
            let difference=(warp(point+axis*0.01,0x7314a5d3).x-warp(point-axis*0.01,0x7314a5d3).x)/0.02;
            assert!((difference-derivative).abs()<0.0005,"{difference} != {derivative}");
        }
    }
}

#[test]
fn rotated_normal_slopes_follow_actual_albedo_gradient_and_green_sign() {
    let n=Vec3::Z;let gu=Vec3::new(1.0,0.12,0.0);let gv=Vec3::new(-0.08,1.0,0.0);
    for i in -8..8 {
        let(r,offset)=variant(i);let p=Vec3::new(0.3,-0.7,0.0);
        let uv=|p:Vec3| r*Vec2::new(gu.dot(p),gv.dot(p))+offset;
        // Independent scalar bump with known texture derivatives.
        let height=|p:Vec3| {let q=uv(p);(q.x*2.3).sin()*0.1+(q.y*1.7).cos()*0.12};
        let q=uv(p);let du=(q.x*2.3).cos()*0.23;let dv=-(q.y*1.7).sin()*0.204;
        let tangent_normal=Vec3::new(du,-dv,1.0).normalize();
        let ru=gu*r.x_axis.x+gv*r.y_axis.x;let rv=gu*r.x_axis.y+gv*r.y_axis.y;
        let gradient=ru*tangent_normal.x-rv*tangent_normal.y;
        let actual=(n*tangent_normal.z+gradient-n*n.dot(gradient)).normalize();
        let eps=0.001;let dx=(height(p+Vec3::X*eps)-height(p-Vec3::X*eps))/(2.0*eps);
        let dy=(height(p+Vec3::Y*eps)-height(p-Vec3::Y*eps))/(2.0*eps);
        let expected=Vec3::new(dx,dy,1.0).normalize();
        assert!(actual.dot(expected)>0.999999,"rotation {i}: {actual:?} vs {expected:?}");
    }
}

#[test]
fn variants_change_the_old_tile_phase_without_camera_inputs() {
    let selector=|p:Vec3| (noise((Vec2::new(p.x,p.z)+Vec2::new(0.37,-0.23)*p.y)/17.0,0x3c6ef372).x*0.5+0.5)*12.0;
    let mut changed=0;
    for x in -30..30 {
        let p=Vec3::new(x as f32*2.7,11.3,-92.1);
        let q=p+Vec3::X*3.7;
        changed+=usize::from((mapped(selector(p),Vec2::new(p.x,p.y)/3.7)-mapped(selector(q),Vec2::new(q.x,q.y)/3.7)).length()>0.2);
    }
    assert!(changed>50,"old one-tile repeats must be decorrelated");
    let shader=include_str!("shaders/shade.wgsl");
    let section=&shader[shader.find("fn rock_mapping(").unwrap()..shader.find("fn hash2(").unwrap()];
    assert!(!section.contains("view.cam")&&!section.contains("distance("));
    assert!(section.contains("variant.turn * dx")&&section.contains("variant.turn * dy"));
}
