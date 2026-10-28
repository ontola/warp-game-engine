//! A compact, comparable listing of a room description: one line per drawn
//! instance (mesh type, world bounds, material), light and text tag. The
//! Godot export and the Rust-built room must give the same listing.

use crate::dump::Dump;
use glam::{Mat4, Vec3};
use serde_json::Value;
use std::fmt::Write;

fn f(x: f64, digits: i32) -> String {
    let k = 10f64.powi(digits);
    let r = (x * k).round() / k;
    let r = if r == 0.0 { 0.0 } else { r };
    format!("{r:.*}", digits as usize)
}

fn v3(v: Vec3) -> String {
    format!("({},{},{})", f(v.x as f64, 3), f(v.y as f64, 3), f(v.z as f64, 3))
}

fn col(c: &Value) -> String {
    let g = |i: usize| f(c[i].as_f64().unwrap_or(0.0), 3);
    format!("({},{},{},{})", g(0), g(1), g(2), g(3))
}

fn num(v: &Value, k: &str) -> f64 {
    v[k].as_f64().unwrap_or(0.0)
}

/// `skip`: instances that are not drawn (Godot exports only visible nodes).
pub fn listing(dump: &Dump, bin: &[u8], skip: &[usize]) -> String {
    let mut out = String::new();
    let floats = |r: [usize; 2]| -> Vec<f32> { bin[r[0]..r[0] + r[1]].chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect() };
    for (i, inst) in dump.instances.iter().enumerate() {
        if skip.contains(&i) {
            continue;
        }
        let mesh = &dump.meshes[inst.mesh];
        let xf = Mat4::from_cols_array(&inst.transform);
        let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        let mut local = Vec3::ZERO;
        let mut verts = 0;
        for s in &mesh.surfaces {
            let p = floats(s.positions);
            verts += p.len() / 3;
            for c in p.chunks_exact(3) {
                let v = Vec3::new(c[0], c[1], c[2]);
                local = local.max(v.abs());
                let w = xf.transform_point3(v);
                lo = lo.min(w);
                hi = hi.max(w);
            }
        }
        let bounds = if (local.x - 0.18).abs() < 1e-4 && (local.y - 0.175).abs() < 1e-4 {
            // A moving head: its aim follows the clock.
            format!("at={}", v3(xf.w_axis.truncate()))
        } else {
            format!("lo={} hi={}", v3(lo), v3(hi))
        };
        let m = inst.materials.first().copied().unwrap_or(-1);
        let material = if m >= 0 {
            let d = &dump.materials[m as usize];
            let tex = |k: &str| {
                let t = d[k].as_i64().unwrap_or(-1);
                if t < 0 { "-".to_string() } else {
                    let t = &dump.textures[t as usize];
                    format!("{}{}x{}", if t.kind == "viewport" { "vp" } else { "img" }, t.width, t.height)
                }
            };
            let emission = d["emission_enabled"].as_bool().unwrap_or(false);
            format!(
                "a={} tex={} r{} m{} e{} u{} t{} c{} f{} uv({},{})",
                col(&d["albedo"]),
                tex("albedo_texture"),
                f(num(d, "roughness"), 2),
                f(num(d, "metallic"), 2),
                if emission { format!("={}x{}/{}", col(&d["emission"]), f(num(d, "emission_energy"), 2), tex("emission_texture")) } else { "-".into() },
                d["unshaded"].as_bool().unwrap_or(false) as u8,
                d["transparency"].as_i64().unwrap_or(0),
                d["cull"].as_i64().unwrap_or(0),
                d["filter"].as_i64().unwrap_or(0),
                f(d["uv1_scale"][0].as_f64().unwrap_or(1.0), 2),
                f(d["uv1_scale"][1].as_f64().unwrap_or(1.0), 2),
            )
        } else {
            "-".into()
        };
        let _ = writeln!(out, "{} v{verts} {bounds} {material} sh{}", mesh.kind, inst.shadow as u8);
    }
    for l in &dump.lights {
        let xf = crate::dump::mat4(&l["transform"]);
        let angle = num(l, "angle");
        let dir = if angle == 6.0 { "moving".into() } else { v3(-xf.z_axis.truncate().normalize()) };
        let pos = if angle == 6.0 { v3(xf.w_axis.truncate()) } else { v3(xf.w_axis.truncate()) };
        let _ = writeln!(
            out,
            "{} p={pos} d={dir} c={} e{} r{} a{} aa{} at{} fog{} sh{}",
            l["type"].as_str().unwrap_or(""),
            col(&l["color"]),
            f(num(l, "energy"), 2),
            f(num(l, "range"), 2),
            f(angle, 2),
            f(num(l, "angle_attenuation"), 2),
            f(num(l, "attenuation"), 2),
            f(num(l, "fog"), 2),
            l["shadow"].as_bool().unwrap_or(false) as u8,
        );
    }
    for l in &dump.labels {
        let xf = crate::dump::mat4(&l["transform"]);
        let _ = writeln!(
            out,
            "label {:?} p={} size{} px{} mod={} out{}",
            l["text"].as_str().unwrap_or(""),
            v3(xf.w_axis.truncate()),
            l["font_size"].as_i64().unwrap_or(0),
            f(num(l, "pixel_size"), 4),
            col(&l["modulate"]),
            l["outline_size"].as_i64().unwrap_or(0),
        );
    }
    out
}
