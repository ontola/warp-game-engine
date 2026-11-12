//! Gas dynamics for the exhaust, after engine-sim's newer high-resolution
//! solvers: the sound is the flow leaving the tailpipe of a simulated
//! engine.
//!
//! - Cylinders are 0D volumes over a slider-crank: compression and
//!   expansion, Wiebe heat release from the trapped charge, cycle-to-cycle
//!   variation, fuel cut on the overrun.
//! - Valves are compressible orifices (choked or not) whose curtain area
//!   follows a cam profile; intake comes from a plenum at manifold pressure
//!   (throttle, boost).
//! - Each header and the main pipe solve the 1D Euler equations with a
//!   finite-volume HLL scheme, MUSCL-reconstructed (minmod) for sharp
//!   pulses. Headers end in a collector volume; a turbine is a restriction.
//! - The open tailpipe is a pressure boundary. What radiates is the rate of
//!   change of the mass flow leaving it (a monopole).
//!
//! The engine's speed is imposed by the game (its drivetrain owns rpm),
//! so this is an acoustic model, not a second engine simulation.

use crate::dsp::Noise;
use serde::Deserialize;

const GAMMA: f32 = 1.35;
const R: f32 = 287.0;
const CV: f32 = R / (GAMMA - 1.0);
const CP: f32 = CV + R;
const P_ATM: f32 = 101_325.0;
const T_ATM: f32 = 300.0;
/// Heat released per kg of trapped air at stoichiometric fuel (J/kg).
const Q_AIR: f32 = 2.9e6;

/// Engine geometry, in Sideways' data/sounds.luau under `x.fluid`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct Geometry {
    /// Swept volume of the whole engine (litres).
    pub displacement: f32,
    pub bore_stroke: f32,
    pub compression: f32,
    pub header_diameter: f32,
    pub pipe_diameter: f32,
    /// Cells per metre in the pipes (the solver substeps to stay stable).
    pub cells_per_m: f32,
    /// Fraction of the pipe area left open by the turbine (1: none).
    pub turbine_area: f32,
    /// Collector volume (litres).
    pub collector: f32,
    /// Cam events, crank degrees after firing TDC: exhaust opens and lasts,
    /// intake opens and lasts.
    pub exhaust_open: f32,
    pub exhaust_duration: f32,
    pub intake_open: f32,
    pub intake_duration: f32,
    /// Ignition, degrees before TDC; the burn lasts `burn` degrees.
    pub spark_advance: f32,
    pub burn: f32,
    /// Runner low-pass (Hz) for this model, replacing the top-level
    /// `runner_cutoff` when set (see EngineSound::runner_cutoff).
    pub runner_cutoff: f32,
}

impl Default for Geometry {
    fn default() -> Self {
        Geometry { displacement: 1.6, bore_stroke: 1.05, compression: 9.5, header_diameter: 0.036, pipe_diameter: 0.063, cells_per_m: 30.0, turbine_area: 0.35, collector: 0.4, exhaust_open: 130.0, exhaust_duration: 250.0, intake_open: 345.0, intake_duration: 245.0, spark_advance: 15.0, burn: 55.0, runner_cutoff: 0.0 }
    }
}

#[derive(Clone, Copy, Default)]
struct Cons {
    rho: f32,
    mom: f32,
    e: f32,
}

#[inline]
fn prim(c: Cons) -> (f32, f32, f32) {
    let rho = c.rho.max(1e-4);
    let u = c.mom / rho;
    let p = ((GAMMA - 1.0) * (c.e - 0.5 * rho * u * u)).max(100.0);
    (rho, u, p)
}

#[inline]
fn cons(rho: f32, u: f32, p: f32) -> Cons {
    Cons { rho, mom: rho * u, e: p / (GAMMA - 1.0) + 0.5 * rho * u * u }
}

#[inline]
fn minmod(a: f32, b: f32) -> f32 {
    if a * b <= 0.0 { 0.0 } else if a.abs() < b.abs() { a } else { b }
}

/// HLL flux between left and right primitive states.
#[inline]
fn hll(l: (f32, f32, f32), r: (f32, f32, f32)) -> Cons {
    let (rl, ul, pl) = l;
    let (rr, ur, pr) = r;
    let cl = (GAMMA * pl / rl).sqrt();
    let cr = (GAMMA * pr / rr).sqrt();
    let sl = (ul - cl).min(ur - cr);
    let sr = (ul + cl).max(ur + cr);
    let el = pl / (GAMMA - 1.0) + 0.5 * rl * ul * ul;
    let er = pr / (GAMMA - 1.0) + 0.5 * rr * ur * ur;
    let fl = Cons { rho: rl * ul, mom: rl * ul * ul + pl, e: ul * (el + pl) };
    if sl >= 0.0 {
        return fl;
    }
    let fr = Cons { rho: rr * ur, mom: rr * ur * ur + pr, e: ur * (er + pr) };
    if sr <= 0.0 {
        return fr;
    }
    let k = 1.0 / (sr - sl);
    Cons {
        rho: (sr * fl.rho - sl * fr.rho + sl * sr * (rr - rl)) * k,
        mom: (sr * fl.mom - sl * fr.mom + sl * sr * (rr * ur - rl * ul)) * k,
        e: (sr * fl.e - sl * fr.e + sl * sr * (er - el)) * k,
    }
}

/// A straight pipe of constant area, cells left to right.
struct Pipe {
    cells: Vec<Cons>,
    flux: Vec<Cons>,
    prim: Vec<(f32, f32, f32)>,
    /// Fastest signal (|u| + c) seen in the last flux pass.
    speed: f32,
    area: f32,
    dx: f32,
}

impl Pipe {
    fn new(len: f32, diameter: f32, per_m: f32, t: f32) -> Pipe {
        let n = ((len * per_m).round() as usize).max(4);
        let rho = P_ATM / (R * t);
        Pipe { cells: vec![cons(rho, 0.0, P_ATM); n], flux: vec![Cons::default(); n + 1], prim: vec![(rho, 0.0, P_ATM); n], speed: 400.0, area: std::f32::consts::PI * diameter * diameter / 4.0, dx: len / n as f32 }
    }

    /// Interior fluxes (faces 1..n-1), MUSCL with minmod on primitives.
    fn interior_fluxes(&mut self) {
        let n = self.cells.len();
        let mut speed = 0.0f32;
        for (d, c) in self.prim.iter_mut().zip(&self.cells) {
            *d = prim(*c);
            speed = speed.max(d.1.abs() + (GAMMA * d.2 / d.0).sqrt());
        }
        self.speed = speed;
        let p = &self.prim;
        let slope = |i: usize| -> (f32, f32, f32) {
            if i == 0 || i + 1 >= n {
                return (0.0, 0.0, 0.0);
            }
            let (a, b, c) = (p[i - 1], p[i], p[i + 1]);
            (minmod(b.0 - a.0, c.0 - b.0), minmod(b.1 - a.1, c.1 - b.1), minmod(b.2 - a.2, c.2 - b.2))
        };
        let mut s_prev = slope(0);
        for f in 1..n {
            let s_next = slope(f);
            let l = p[f - 1];
            let r = p[f];
            let l = ((l.0 + 0.5 * s_prev.0).max(1e-3), l.1 + 0.5 * s_prev.1, (l.2 + 0.5 * s_prev.2).max(100.0));
            let r = ((r.0 - 0.5 * s_next.0).max(1e-3), r.1 - 0.5 * s_next.1, (r.2 - 0.5 * s_next.2).max(100.0));
            self.flux[f] = hll(l, r);
            s_prev = s_next;
        }
    }

    fn update(&mut self, dt: f32) {
        let k = dt / self.dx;
        for i in 0..self.cells.len() {
            let (a, b) = (self.flux[i], self.flux[i + 1]);
            let c = &mut self.cells[i];
            c.rho -= k * (b.rho - a.rho);
            c.mom -= k * (b.mom - a.mom);
            c.e -= k * (b.e - a.e);
            if c.rho < 1e-3 {
                *c = cons(1e-3, 0.0, 1000.0);
            }
        }
    }
}

/// Flux through a face from a mass flow `mdot` (kg/s, positive into the
/// pipe from the left) with upwind total enthalpy `h0`, against a wall
/// pressure `p_wall`, per unit pipe area.
#[inline]
fn face_flux(mdot: f32, area: f32, rho_up: f32, h0: f32, p_wall: f32) -> Cons {
    let g = mdot / area;
    let u = g / rho_up.max(1e-3);
    Cons { rho: g, mom: g * u + p_wall, e: g * h0 }
}

/// Compressible orifice: mass flow (kg/s) from (p1, t1) to p2 through
/// effective area `a`; negative when p2 > p1 (then from side 2, at t2).
#[inline]
fn orifice(a: f32, p1: f32, t1: f32, p2: f32, t2: f32) -> f32 {
    if a <= 0.0 {
        return 0.0;
    }
    let (pu, tu, pd, sign) = if p1 >= p2 { (p1, t1, p2, 1.0) } else { (p2, t2, p1, -1.0) };
    let pr = (pd / pu).max(0.528); // choked below the critical ratio
    let flow = (2.0 * GAMMA / (GAMMA - 1.0) * (pr.powf(2.0 / GAMMA) - pr.powf((GAMMA + 1.0) / GAMMA))).max(0.0).sqrt();
    sign * a * pu / (R * tu).sqrt() * flow
}

/// Valve curtain area over the event (0..1 of its duration), sin^2 cam.
#[inline]
fn lift(x: f32) -> f32 {
    if !(0.0..=1.0).contains(&x) { 0.0 } else { (x * std::f32::consts::PI).sin().powi(2) }
}

struct Cyl {
    phase: f32,
    m: f32,
    t: f32,
    v: f32,
    q_total: f32,
    last: f32,
}

pub struct FluidEngine {
    geo: Geometry,
    cyl: Vec<Cyl>,
    headers: Vec<Pipe>,
    pipe: Pipe,
    col_m: f32,
    col_t: f32,
    col_v: f32,
    sr: f32,
    crank_r: f32,
    rod: f32,
    piston_area: f32,
    v_clear: f32,
    valve_area: [f32; 2],
    crank: f32,
    noise: Noise,
    last_exit: f32,
    /// Cell updates done, for cost reports.
    pub work: u64,
    /// Exhaust runner pressure per cylinder after the last sample (bar
    /// above ambient, plus engine-sim's 0.1 x dynamic pressure).
    pub runner: Vec<f32>,
}

impl FluidEngine {
    pub fn new(geo: Geometry, phases: &[f32], headers: &[f32], pipe_len: f32, sr: f32, seed: u32) -> FluidEngine {
        let n = phases.len().max(1);
        let swept = geo.displacement * 1e-3 / n as f32;
        // swept = pi/4 b^2 s, s = b / bore_stroke
        let bore = (4.0 * swept * geo.bore_stroke / std::f32::consts::PI).cbrt();
        let stroke = bore / geo.bore_stroke;
        let piston_area = std::f32::consts::PI * bore * bore / 4.0;
        let v_clear = swept / (geo.compression - 1.0);
        // Two valves per side, curtain pi d L with d ~ 0.36 bore, L ~ 0.25 d.
        let d_in = 0.38 * bore;
        let d_ex = 0.33 * bore;
        let valve_area = [2.0 * std::f32::consts::PI * d_in * d_in * 0.25 * 0.7, 2.0 * std::f32::consts::PI * d_ex * d_ex * 0.25 * 0.7];
        let cyl = phases
            .iter()
            .map(|&ph| Cyl { phase: ph, m: P_ATM * swept / (R * T_ATM), t: T_ATM, v: swept, q_total: 0.0, last: 0.0 })
            .collect();
        let hot = 800.0;
        let headers = headers.iter().map(|&l| Pipe::new(l, geo.header_diameter, geo.cells_per_m, hot)).collect();
        let pipe = Pipe::new(pipe_len, geo.pipe_diameter, geo.cells_per_m, hot);
        let col_v = geo.collector * 1e-3;
        FluidEngine {
            cyl,
            headers,
            pipe,
            col_m: P_ATM * col_v / (R * hot),
            col_t: hot,
            col_v,
            sr,
            crank_r: stroke / 2.0,
            rod: stroke * 1.6,
            piston_area,
            v_clear,
            valve_area,
            crank: 0.0,
            noise: Noise::new(seed ^ 0xF1D),
            last_exit: 0.0,
            work: 0,
            runner: vec![0.0; n],
            geo,
        }
    }

    fn volume(&self, a_deg: f32) -> f32 {
        let a = a_deg.to_radians();
        let (s, c) = a.sin_cos();
        let x = self.crank_r * c + (self.rod * self.rod - self.crank_r * self.crank_r * s * s).sqrt();
        self.v_clear + self.piston_area * (self.rod + self.crank_r - x)
    }

    /// State for debugging: crank, cylinder 0 (p, T, m), first header cell p,
    /// collector p, last pipe cell (p, u).
    pub fn probe(&self) -> [f32; 8] {
        let c = &self.cyl[0];
        let (_, _, hp) = prim(self.headers[0].cells[0]);
        let n = self.pipe.cells.len();
        let (_, u, pp) = prim(self.pipe.cells[n - 1]);
        [self.crank, c.m * R * c.t / c.v, c.t, c.m * 1e6, hp, self.col_m * R * self.col_t / self.col_v, pp, u]
    }

    /// Advance one audio sample; returns the radiated signal (arbitrary
    /// units, about +-1 at full load).
    pub fn sample(&mut self, rpm: f32, throttle: f32, boost: f32, fuel: bool, rough: f32) -> f32 {
        let dt_audio = 1.0 / self.sr;
        // Substeps for the pipes' CFL condition, from the speeds of the last
        // pass with a margin (they change little within a sample).
        let speed = self.pipe.speed.max(self.headers.iter().map(|h| h.speed).fold(0.0, f32::max));
        let dx = self.pipe.dx.min(self.headers.iter().map(|h| h.dx).fold(f32::MAX, f32::min));
        let steps = ((speed * 1.2 * dt_audio / (0.7 * dx)).ceil() as usize).clamp(1, 8);
        let dt = dt_audio / steps as f32;
        let mut exit = 0.0;
        let p_man = P_ATM * (0.25 + 0.75 * throttle) * (1.0 + boost * 1.1);
        let t_man = T_ATM + boost * 40.0;
        for _ in 0..steps {
            let dcrank = rpm / 60.0 * 360.0 * dt;
            self.crank = (self.crank + dcrank).rem_euclid(720.0);
            // Cylinders: kinematics, valves, combustion.
            let n_cyl = self.cyl.len();
            for k in 0..n_cyl {
                let a = (self.crank - self.cyl[k].phase).rem_euclid(720.0);
                let v_new = self.volume(a);
                let (ex_area, in_area) = (self.valve_area[1] * lift((a - self.geo.exhaust_open) / self.geo.exhaust_duration), self.valve_area[0] * lift((a - self.geo.intake_open) / self.geo.intake_duration));
                let spark = 720.0 - self.geo.spark_advance;
                let hp = &self.headers[k];
                let (hr, hu, hpres) = prim(hp.cells[0]);
                let ht = hpres / (hr * R);
                let c = &mut self.cyl[k];
                // Ignition: the charge trapped at intake closing burns from the spark.
                if c.last < spark && a >= spark {
                    let fuel_mass = if fuel { 1.0 } else { 0.0 };
                    c.q_total = c.m * Q_AIR * 0.38 * fuel_mass * (1.0 + rough * self.noise.gauss()).max(0.3);
                }
                c.last = a;
                let p = c.m * R * c.t / c.v;
                // Wiebe heat release over `burn` degrees from the spark.
                let theta = (a + self.geo.spark_advance).rem_euclid(720.0);
                let burn = self.geo.burn.max(1.0);
                let mut dq = 0.0;
                if theta < burn && c.q_total > 0.0 {
                    let xb = 1.0 - (-5.0 * (theta / burn).powi(3)).exp();
                    let xb_next = 1.0 - (-5.0 * ((theta + dcrank) / burn).min(1.0).powi(3)).exp();
                    dq = c.q_total * (xb_next - xb).max(0.0);
                }
                // Valve flows (positive out of the cylinder).
                let m_ex = orifice(ex_area, p, c.t, hpres, ht);
                let m_in = orifice(in_area, p, c.t, p_man, t_man);
                let h_out_ex = if m_ex > 0.0 { CP * c.t } else { CP * ht + 0.5 * hu * hu };
                let h_out_in = if m_in > 0.0 { CP * c.t } else { CP * t_man };
                let mut dm = -(m_ex + m_in) * dt;
                dm = dm.max(-0.9 * c.m);
                let du = -p * (v_new - c.v) + dq - (m_ex * h_out_ex + m_in * h_out_in) * dt;
                let u = c.m * CV * c.t + du;
                c.m = (c.m + dm).max(1e-7);
                c.t = (u / (c.m * CV)).clamp(250.0, 3200.0);
                c.v = v_new;
                // Header inlet face: the exhaust valve.
                let h = &mut self.headers[k];
                let rho_up = if m_ex > 0.0 { p / (R * c.t) } else { hr };
                h.flux[0] = face_flux(m_ex, h.area, rho_up, h_out_ex, hpres);
            }
            // Header outlets into the collector.
            let col_p = self.col_m * R * self.col_t / self.col_v;
            let mut col_dm = 0.0;
            let mut col_de = 0.0;
            for h in self.headers.iter_mut() {
                let n = h.cells.len();
                let (r, u, p) = prim(h.cells[n - 1]);
                let t = p / (r * R);
                let md = orifice(h.area * 0.9, p, t, col_p, self.col_t);
                let h0 = if md > 0.0 { CP * t + 0.5 * u * u } else { CP * self.col_t };
                let rho_up = if md > 0.0 { r } else { col_p / (R * self.col_t) };
                h.flux[n] = face_flux(md, h.area, rho_up, h0, p);
                col_dm += md * dt;
                col_de += md * h0 * dt;
            }
            // Collector into the main pipe, through the turbine.
            let (pr0, pu0, pp0) = prim(self.pipe.cells[0]);
            let pt0 = pp0 / (pr0 * R);
            let md = orifice(self.pipe.area * self.geo.turbine_area, col_p, self.col_t, pp0, pt0);
            let h0 = if md > 0.0 { CP * self.col_t } else { CP * pt0 + 0.5 * pu0 * pu0 };
            let rho_up = if md > 0.0 { col_p / (R * self.col_t) } else { pr0 };
            self.pipe.flux[0] = face_flux(md, self.pipe.area, rho_up, h0, pp0);
            col_dm -= md * dt;
            col_de -= md * h0 * dt;
            let u_col = self.col_m * CV * self.col_t + col_de;
            self.col_m = (self.col_m + col_dm).max(1e-6);
            self.col_t = (u_col / (self.col_m * CV)).clamp(250.0, 2500.0);
            // Open end: the atmosphere just outside.
            let n = self.pipe.cells.len();
            let (r, u, p) = prim(self.pipe.cells[n - 1]);
            // Subsonic open end: atmospheric pressure, density from the cell
            // along an isentrope, the same for outflow and backflow (a jump
            // to cold air when the flow reverses clicks).
            let out_state = (r * (P_ATM / p.max(1.0)).powf(1.0 / GAMMA), u, P_ATM);
            self.pipe.flux[n] = hll((r, u, p), out_state);
            exit = self.pipe.flux[n].rho * self.pipe.area;
            // Pipes.
            for h in self.headers.iter_mut() {
                h.interior_fluxes();
                h.update(dt);
                self.work += h.cells.len() as u64;
            }
            self.pipe.interior_fluxes();
            self.pipe.update(dt);
            self.work += self.pipe.cells.len() as u64;
        }
        for (r, h) in self.runner.iter_mut().zip(&self.headers) {
            let (rho, u, p) = prim(h.cells[0]);
            *r = (p - P_ATM + 0.1 * rho * u * u) * 1e-5;
        }
        // Monopole radiation: d(mass flow)/dt.
        let d = (exit - self.last_exit) * self.sr;
        self.last_exit = exit;
        d * 1.5e-3
    }
}
