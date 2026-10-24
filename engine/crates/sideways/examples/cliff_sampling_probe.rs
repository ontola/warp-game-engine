//! CPU-only source-grid diagnosis of r15d captures 57532-1 and 57532-2.
use gdcompat::{Pcg,math::v3};
use serde_json::json;
use sideways::stage::{Stage,RUNOUT};

fn triangle(s:&Stage,x:f64,z:f64,pitch:f64)->f64 {
    let gx=(x/pitch).floor();let gz=(z/pitch).floor();let u=x/pitch-gx;let v=z/pitch-gz;
    let h=|i:f64,j:f64|s.height((gx+i)*pitch,(gz+j)*pitch);
    if u+v<=1.0 {h(0.,0.)*(1.-u-v)+h(1.,0.)*u+h(0.,1.)*v}
    else {h(1.,1.)*(u+v-1.)+h(1.,0.)*(1.-v)+h(0.,1.)*(1.-u)}
}
fn stats(mut a:Vec<f64>)->serde_json::Value {a.sort_by(f64::total_cmp);json!({"n":a.len(),"mean":a.iter().sum::<f64>()/a.len()as f64,"p95":a[a.len()*95/100],"max":a.last()})}
fn main() {
    let mut rng=Pcg::new(1);rng.next_u32();let mut s=Stage::generate(&mut rng,3000.0+RUNOUT);
    if std::env::args().any(|s|s=="--dry") {s.rivers.clear();}
    assert_eq!((s.noise_seed,s.detail_seed),(683407368,2033395789));
    let mut reports=Vec::new();
    for (name,point) in [("capture1",v3(40.455135,8.250108,-1429.9132)),("capture2",v3(9.212948,-7.2513,-1899.1685))] {
        let at=s.locate(point,-1).index as usize;let mut variants=Vec::new();
        for pitch in [2.0,1.0,0.5,0.25] {
            let mut errors=Vec::new();let mut worst=json!(null);let mut max_error=0.;
            for row in at.saturating_sub(45)..(at+46).min(s.points.len()) {
                let p=s.points[row];let d=s.dirs[row];let side=s.ledges[row].signum();if side==0. {continue}
                for k in 0..=100 {
                    let off=s.widths[row]as f64+1.0+k as f64*0.1;
                    let x=p.x as f64-d.z as f64*side as f64*off;
                    let z=p.z as f64+d.x as f64*side as f64*off;
                    let analytical=s.height(x,z);let raster=triangle(&s,x,z,pitch);let error=(raster-analytical).abs();errors.push(error);
                    if error>max_error {max_error=error;worst=json!({"row":row,"position":[x,analytical,z],"raster_height":raster,"road_offset_m":off-s.widths[row]as f64});}
                }
            }
            variants.push(json!({"pitch_m":pitch,"vertical_error_m":stats(errors),"worst":worst}));
        }
        reports.push(json!({"capture":name,"row":at,"road_position":[s.points[at].x,s.points[at].y,s.points[at].z],"ledge":s.ledges[at],"wall_height_m":s.ledge_wall[at],"variants":variants}));
    }
    println!("{}",serde_json::to_string_pretty(&json!({"seeds":[s.noise_seed,s.detail_seed],"source_grid_only":true,"reports":reports})).unwrap());
}
