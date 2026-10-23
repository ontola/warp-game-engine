//! CPU-only diagnosis of the actual seed-1 / round-2 F12 terrain report.
//! Run from engine: cargo run -p sideways --example terrain_foundation_probe -- <out.json>
use gdcompat::{Pcg,math::v3};
use serde_json::json;
use sideways::{builder,stage::{Stage,RUNOUT}};
use std::collections::BTreeMap;

fn actual_stage() -> Stage {
    // Recorded stage seeds are consecutive draws 835 and 836 of Pcg(seed=1).
    // Replay that state directly: no physics, rendering, car assets or window.
    let mut rng=Pcg::new(1);rng.skip(835);
    let s=Stage::generate(&mut rng,3000.0+RUNOUT);
    assert_eq!((s.noise_seed,s.detail_seed),(370671355,-305494282));
    s
}

fn main() -> Result<(),Box<dyn std::error::Error>> {
    let s=actual_stage();let mut dry=actual_stage();dry.rivers.clear();
    let q=v3(-57.27709198,11.44343472,-662.73962402);
    let located=s.locate(q,-1);let at=located.index as usize;
    let road=builder::road(&s);let mut cross_sections=Vec::new();
    let mut max_edge_drop=0.0f64;let mut max_carve_drop=0.0f64;
    let mut chunks=BTreeMap::new();
    for i in at.saturating_sub(65)..(at+66).min(s.points.len()) {
        let p=s.points[i];let d=s.dirs[i];let right=v3(-d.z,0.0,d.x);
        let mut samples=Vec::new();
        for side in [-1.0,0.0,1.0] {
            let a=p+right*(side*(s.widths[i]+0.8));
            let raw=dry.height_near(a.x as f64,a.z as f64,i as i64);
            let carved=s.height_near(a.x as f64,a.z as f64,i as i64);
            let nearest=s.nearest_index(a.x as f64,a.z as f64);
            let global=s.height(a.x as f64,a.z as f64);
            max_carve_drop=max_carve_drop.max(raw-carved);
            if side!=0.0 {max_edge_drop=max_edge_drop.max(p.y as f64-carved);}
            samples.push(json!({"side":side,"x":a.x,"z":a.z,"raw":raw,"carved":carved,"nearest":nearest,"global":global}));
        }
        let key=((p.x/64.0).floor() as i32,(p.z/64.0).floor() as i32);
        for z in -1..=1 {for x in -1..=1 {chunks.entry((key.0+x,key.1+z)).or_insert_with(||builder::terrain_chunk(&s,(key.0+x,key.1+z)));}}
        cross_sections.push(json!({"index":i,"along":i*2,"point":[p.x,p.y,p.z],"width":s.widths[i],"ledge":s.ledges[i],
            "left_edge":road.verts[i*road.cols].y,"right_edge":road.verts[(i+1)*road.cols-1].y,"samples":samples}));
    }
    let mut mismatches=Vec::new();let mut worst=0.0f64;let mut total_bad=0usize;
    for chunk in chunks.values() {for j in 0..33 {for i in 0..33 {
        let x=chunk.x0+i as f64*2.0;let z=chunk.z0+j as f64*2.0;
        let actual=chunk.heights[j*33+i] as f64;let reference=s.height(x,z);
        let delta=(actual-reference).abs();worst=worst.max(delta);
        if delta>0.05 {
            total_bad+=1;
            mismatches.push((delta,json!({"x":x,"z":z,"chunk":actual,"stage":reference,"delta":delta,
                "nearest":s.nearest_index(x,z),"road_distance":chunk.road_d[j*33+i]})));
        }
    }}}
    mismatches.sort_by(|a,b|b.0.total_cmp(&a.0));mismatches.truncate(24);
    let report=json!({"seeds":[s.noise_seed,s.detail_seed],"replay_pcg_skip":835,
        "car":[q.x,q.y,q.z],"nearest_row":at,"along":located.along,
        "height_under_car":s.height(q.x as f64,q.z as f64),"raw_height_under_car":dry.height(q.x as f64,q.z as f64),
        "river_crossings":s.rivers.iter().map(|r|json!({"cross":r.cross,"along":r.cross*2,"ford":r.ford,"half":r.half,"depth":r.depth,
            "points":r.pts.iter().map(|p|[p.x,p.y]).collect::<Vec<_>>(),"levels":r.levels,"wet":r.wet})).collect::<Vec<_>>(),
        "summary":{"max_road_to_edge_drop":max_edge_drop,"max_river_carve_drop":max_carve_drop,
            "chunk_count":chunks.len(),"max_chunk_vs_global_error":worst,"mismatched_samples_over_5cm":total_bad},
        "worst_chunk_mismatches":mismatches.into_iter().map(|(_,v)|v).collect::<Vec<_>>(),"cross_sections":cross_sections});
    let out=std::env::args().nth(1).ok_or("provide output JSON path")?;
    std::fs::write(&out,serde_json::to_vec_pretty(&report)?)?;
    println!("{}",serde_json::to_string_pretty(&report["summary"])?);
    println!("CPU report: {out}");Ok(())
}
