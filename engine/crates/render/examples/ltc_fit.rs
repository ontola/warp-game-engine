//! Fits the linearly transformed cosines (Heitz, Dupuy, Hill, Neubelt,
//! "Real-Time Polygonal-Light Shading with Linearly Transformed Cosines",
//! SIGGRAPH 2016) to the engine's GGX specular (`ggx` in shade.wgsl:
//! GGX distribution, height-correlated Smith masking, no Fresnel), and
//! writes the tables `src/ltc_ggx.bin` that area-light specular reads
//! (`light_face`, clustered_lights.wgsl).
//!
//!   cargo run --release -p render --example ltc_fit
//!
//! Table: 64 x 64, roughness (perceptual, alpha = roughness^2) along x and
//! sqrt(1 - cos(view angle)) along y. Per entry six f32: the inverse matrix
//! normalised by its middle element (m00, m02, m20, m22, column-major as
//! the shader builds it), then the BRDF's directional albedo without
//! Fresnel and its Schlick weight (the integral of brdf * cos * (1 - v.h)^5).
//! The fit follows the paper's: for each view, the cosine lobe is turned to
//! the BRDF's mean direction and its three free matrix entries are fitted by
//! Nelder-Mead to the cubed difference, importance sampled from both lobes.
use std::f64::consts::PI;

const N: usize = 64;
const SAMPLES: usize = 32;
const MIN_ALPHA: f64 = 1e-5;

type V3 = [f64; 3];
type M3 = [[f64; 3]; 3]; // m[column][row]

fn dot(a: V3, b: V3) -> f64 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn len(a: V3) -> f64 { dot(a, a).sqrt() }
fn norm(a: V3) -> V3 { let l = len(a); [a[0] / l, a[1] / l, a[2] / l] }
fn mul(m: &M3, v: V3) -> V3 {
    [m[0][0] * v[0] + m[1][0] * v[1] + m[2][0] * v[2], m[0][1] * v[0] + m[1][1] * v[1] + m[2][1] * v[2], m[0][2] * v[0] + m[1][2] * v[1] + m[2][2] * v[2]]
}
fn mat_mul(a: &M3, b: &M3) -> M3 { let mut r = [[0.0; 3]; 3]; for c in 0..3 { r[c] = mul(a, b[c]); } r }
fn det(m: &M3) -> f64 {
    m[0][0] * (m[1][1] * m[2][2] - m[2][1] * m[1][2]) - m[1][0] * (m[0][1] * m[2][2] - m[2][1] * m[0][2]) + m[2][0] * (m[0][1] * m[1][2] - m[1][1] * m[0][2])
}
fn inverse(m: &M3) -> M3 {
    let d = det(m);
    let mut r = [[0.0; 3]; 3];
    for c in 0..3 {
        for row in 0..3 {
            // Inverse element (c, row) = cofactor (row, c) / det.
            let (c1, c2) = ((row + 1) % 3, (row + 2) % 3);
            let (r1, r2) = ((c + 1) % 3, (c + 2) % 3);
            r[c][row] = (m[c1][r1] * m[c2][r2] - m[c1][r2] * m[c2][r1]) / d;
        }
    }
    r
}

/// The engine's specular times the cosine, without Fresnel, and the pdf of
/// sampling `l` from the GGX normal distribution.
fn brdf(v: V3, l: V3, alpha: f64) -> (f64, f64) {
    if v[2] <= 0.0 || l[2] <= 0.0 { return (0.0, 0.0); }
    let h = norm([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
    let a2 = alpha * alpha;
    let nh = h[2];
    let d = a2 / (PI * (nh * nh * (a2 - 1.0) + 1.0).powi(2));
    let lambda = |z: f64| { let t2 = (1.0 - z * z) / (z * z); (-1.0 + (1.0 + a2 * t2).sqrt()) * 0.5 };
    let g2 = 1.0 / (1.0 + lambda(v[2]) + lambda(l[2]));
    let value = d * g2 / (4.0 * v[2]);
    let pdf = d * nh / (4.0 * dot(v, h));
    (value, pdf)
}

fn sample_brdf(v: V3, alpha: f64, u1: f64, u2: f64) -> V3 {
    let phi = 2.0 * PI * u1;
    let r = alpha * (u2 / (1.0 - u2)).sqrt();
    let h = norm([r * phi.cos(), r * phi.sin(), 1.0]);
    let vh = dot(v, h);
    [2.0 * vh * h[0] - v[0], 2.0 * vh * h[1] - v[1], 2.0 * vh * h[2] - v[2]]
}

#[derive(Clone, Copy)]
struct Ltc { m11: f64, m22: f64, m13: f64, x: V3, y: V3, z: V3, amplitude: f64, m: M3, inv: M3, det: f64 }

impl Ltc {
    fn update(&mut self) {
        let frame: M3 = [self.x, self.y, self.z];
        let local: M3 = [[self.m11, 0.0, 0.0], [0.0, self.m22, 0.0], [self.m13, 0.0, 1.0]];
        self.m = mat_mul(&frame, &local);
        self.inv = inverse(&self.m);
        self.det = det(&self.inv).abs();
    }
    fn eval(&self, l: V3) -> f64 {
        let lo = mul(&self.inv, l);
        let ll = len(lo);
        let lo = [lo[0] / ll, lo[1] / ll, lo[2] / ll];
        let d = lo[2].max(0.0) / PI;
        self.amplitude * d * self.det / (ll * ll * ll)
    }
    fn sample(&self, u1: f64, u2: f64) -> V3 {
        let theta = u1.sqrt().acos();
        let phi = 2.0 * PI * u2;
        norm(mul(&self.m, [theta.sin() * phi.cos(), theta.sin() * phi.sin(), theta.cos()]))
    }
}

fn error(ltc: &Ltc, v: V3, alpha: f64) -> f64 {
    let mut e = 0.0;
    for j in 0..SAMPLES {
        for i in 0..SAMPLES {
            let u1 = (i as f64 + 0.5) / SAMPLES as f64;
            let u2 = (j as f64 + 0.5) / SAMPLES as f64;
            for l in [ltc.sample(u1, u2), sample_brdf(v, alpha, u1, u2)] {
                let (eb, pb) = brdf(v, l, alpha);
                let el = ltc.eval(l);
                let pl = el / ltc.amplitude;
                if pb + pl > 0.0 { e += (eb - el).abs().powi(3) / (pb + pl); }
            }
        }
    }
    e / (SAMPLES * SAMPLES) as f64
}

/// Directional albedo without Fresnel, its Schlick weight, and the lobe's
/// mean direction.
fn averages(v: V3, alpha: f64) -> (f64, f64, V3) {
    let (mut norm_sum, mut fresnel, mut dir) = (0.0, 0.0, [0.0; 3]);
    for j in 0..SAMPLES {
        for i in 0..SAMPLES {
            let l = sample_brdf(v, alpha, (i as f64 + 0.5) / SAMPLES as f64, (j as f64 + 0.5) / SAMPLES as f64);
            let (e, p) = brdf(v, l, alpha);
            if p <= 0.0 { continue; }
            let w = e / p;
            let h = norm([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
            norm_sum += w;
            fresnel += w * (1.0 - dot(v, h).max(0.0)).powi(5);
            for k in 0..3 { dir[k] += w * l[k]; }
        }
    }
    let n = (SAMPLES * SAMPLES) as f64;
    dir[1] = 0.0;
    (norm_sum / n, fresnel / n, norm(dir))
}

fn nelder_mead(start: [f64; 3], delta: f64, tolerance: f64, max_iter: usize, f: impl Fn([f64; 3]) -> f64) -> [f64; 3] {
    let mut s = [start; 4];
    for i in 0..3 { s[i + 1][i] += delta; }
    let mut fs: [f64; 4] = std::array::from_fn(|i| f(s[i]));
    for _ in 0..max_iter {
        let mut order = [0usize, 1, 2, 3];
        order.sort_by(|&a, &b| fs[a].total_cmp(&fs[b]));
        let (best, worst, second) = (order[0], order[3], order[2]);
        if (fs[worst] - fs[best]).abs() <= tolerance * fs[best].abs().max(1e-300) { break; }
        let mut c = [0.0; 3];
        for &i in &order[..3] { for k in 0..3 { c[k] += s[i][k] / 3.0; } }
        let at = |t: f64| -> [f64; 3] { std::array::from_fn(|k| c[k] + t * (s[worst][k] - c[k])) };
        let r = at(-1.0);
        let fr = f(r);
        if fr < fs[best] {
            let e = at(-2.0);
            let fe = f(e);
            if fe < fr { s[worst] = e; fs[worst] = fe; } else { s[worst] = r; fs[worst] = fr; }
        } else if fr < fs[second] {
            s[worst] = r; fs[worst] = fr;
        } else {
            let k = if fr < fs[worst] { at(-0.5) } else { at(0.5) };
            let fk = f(k);
            if fk < fs[worst].min(fr) { s[worst] = k; fs[worst] = fk; } else {
                for &i in &order[1..] { s[i] = std::array::from_fn(|k| s[best][k] + 0.5 * (s[i][k] - s[best][k])); fs[i] = f(s[i]); }
            }
        }
    }
    let best = (0..4).min_by(|&a, &b| fs[a].total_cmp(&fs[b])).unwrap();
    s[best]
}

fn main() {
    let started = std::time::Instant::now();
    let mut table = vec![[0.0f32; 6]; N * N];
    let mut fits: Vec<Option<Ltc>> = vec![None; N * N];
    for a in (0..N).rev() {
        for t in 0..N {
            let x = t as f64 / (N - 1) as f64;
            let ct = 1.0 - x * x;
            let theta = ct.acos().min(PI / 2.0 - 1e-4);
            let v = [theta.sin(), 0.0, theta.cos()];
            let rough = a as f64 / (N - 1) as f64;
            let alpha = (rough * rough).max(MIN_ALPHA);
            let (amp, fresnel, mean) = averages(v, alpha);
            let mut ltc = if t == 0 {
                // Normal incidence: isotropic, started from the rougher fit.
                let prev = if a + 1 < N { fits[(a + 1) + t * N] } else { None };
                let m = prev.map_or(1.0, |p| p.m11);
                Ltc { m11: m, m22: m, m13: 0.0, x: [1.0, 0.0, 0.0], y: [0.0, 1.0, 0.0], z: [0.0, 0.0, 1.0], amplitude: amp, m: [[0.0; 3]; 3], inv: [[0.0; 3]; 3], det: 1.0 }
            } else {
                let prev = fits[a + (t - 1) * N].unwrap();
                Ltc { x: [mean[2], 0.0, -mean[0]], y: [0.0, 1.0, 0.0], z: mean, amplitude: amp, ..prev }
            };
            ltc.update();
            let isotropic = t == 0;
            let apply = |p: [f64; 3], base: &Ltc| -> Ltc {
                let mut l = *base;
                l.m11 = p[0].max(1e-7);
                l.m22 = if isotropic { l.m11 } else { p[1].max(1e-7) };
                l.m13 = if isotropic { 0.0 } else { p[2] };
                l.update();
                l
            };
            let base = ltc;
            let best = nelder_mead([ltc.m11, ltc.m22, ltc.m13], 0.05, 1e-5, 400, |p| error(&apply(p, &base), v, alpha));
            let ltc = apply(best, &base);
            fits[a + t * N] = Some(ltc);
            if std::env::var("LTC_CHECK").is_ok_and(|c| c == format!("{a},{t}")) { check(a, t, &ltc); }
            let inv = ltc.inv;
            let s = inv[1][1];
            table[a + t * N] = [(inv[0][0] / s) as f32, (inv[0][2] / s) as f32, (inv[2][0] / s) as f32, (inv[2][2] / s) as f32, amp as f32, fresnel as f32];
        }
        eprint!("\rroughness row {:2} done ({:.0} s)", a, started.elapsed().as_secs_f64());
    }
    eprintln!();
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ltc_ggx.bin");
    let bytes: Vec<u8> = table.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
    std::fs::write(&out, &bytes).expect("write table");
    println!("wrote {} ({} bytes) in {:.1} s", out.display(), bytes.len(), started.elapsed().as_secs_f64());
}

#[allow(dead_code)]
fn check(a: usize, t: usize, ltc: &Ltc) {
    let x = t as f64 / (N - 1) as f64;
    let theta = (1.0 - x * x).acos().min(PI / 2.0 - 1e-4);
    let v = [theta.sin(), 0.0, theta.cos()];
    let rough = a as f64 / (N - 1) as f64;
    let alpha = (rough * rough).max(MIN_ALPHA);
    eprintln!("check a={a} t={t} theta={:.1}deg m11={:.4} m22={:.4} m13={:.4} amp={:.4}", theta.to_degrees(), ltc.m11, ltc.m22, ltc.m13, ltc.amplitude);
    for k in -8..=8 {
        let phi = k as f64 * 10.0f64.to_radians();
        let l = [-phi.sin(), 0.0, phi.cos()];
        let (b, _) = brdf(v, l, alpha);
        eprintln!("  l at {:4} deg (+ = toward reflection): brdf {:.4} ltc {:.4}", k * 10, b, ltc.eval(l));
    }
}
