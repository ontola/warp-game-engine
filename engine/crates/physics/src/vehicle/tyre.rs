//! A tyre built from measured characteristic values, in Pacejka's Magic
//! Formula form y = D sin(C atan(Bx - E(Bx - atan Bx))).
//!
//! Pacejka (Tire and Vehicle Dynamics, sec. 4.3) derives B, C, D and E from
//! four measurable numbers per direction: the slip stiffness K (slope at
//! zero), the peak D, the slip at the peak, and the sliding level (the
//! force at large slip as a share of the peak). Here:
//!
//! - K, the peak and its load sensitivity, and the slip at the peak come from
//!   a published MF 5.2 property file for a 205/60R15 91V passenger tyre at
//!   2.2 bar (header "Tire MF52 205/60R15 91V, 2.2bar", 23/07/2010, quoted on
//!   eng-tips thread 348016): at 3000 N it has cornering stiffness 19.2 x load
//!   per radian and peaks at mu 1.12 at 9.1 degrees; longitudinally slip
//!   stiffness 27.6 x load and a peak of 1.21 at slip ratio 0.144.
//! - The sliding level comes from measured peak and sliding friction per
//!   surface (Wong, Theory of Ground Vehicles, 1993, p. 26), see
//!   `surfaces.rs`. The property file itself is not used past its peak: its
//!   lateral fit (C = 2.13) only holds near the peak and turns negative at
//!   large slip angles under high load.
//!
//! Combined slip uses Pacejka's similarity method: slip ratio and slip
//! angle are each divided by their peak slip, the length of that vector
//! picks the point on each curve, its direction splits the force.

pub const FNOMIN: f64 = 6500.0;
const FZMIN: f64 = 400.0;
const FZMAX: f64 = 8500.0;

const PDX1: f64 = 1.1703;
const PDX2: f64 = -0.081328;
const PKX1: f64 = 36.411;
const PKX2: f64 = 0.12615;
const PKX3: f64 = 0.51289;
const PDY1: f64 = 1.0283;
const PDY2: f64 = -0.16758;
const PKY1: f64 = -20.505;
const PKY2: f64 = 2.0284;
// Pneumatic trail (aligning torque) from the same file.
const R0: f64 = 0.3185;
const QBZ1: f64 = 19.588;
const QCZ1: f64 = 1.2012;
const QDZ1: f64 = 0.12819;
const QEZ1: f64 = -0.0044308;

/// Slip at the peak of the pure-slip curves at 3000 N on dry asphalt
/// (computed from the property file).
pub const KAPPA_PEAK: f64 = 0.144;
pub const ALPHA_PEAK: f64 = 0.159;
/// Dry asphalt's sliding level, Wong: sliding 0.75 over peak 0.85.
pub const DRY_SLIDING: f64 = 0.75 / 0.85;
const REF_LOAD: f64 = 3000.0;

/// Scale factors for one wheel on one surface.
#[derive(Clone, Copy, Debug)]
pub struct Scale {
    /// Peak friction relative to dry asphalt (MF LMUX = LMUY).
    pub mu: f64,
    /// Sliding friction as a share of the peak on this surface.
    pub sliding: f64,
    /// Slip stiffness scale (MF LKX).
    pub kx: f64,
    /// Slip stiffness scale (MF LKY).
    pub ky: f64,
}

impl Scale {
    pub const DRY: Scale = Scale { mu: 1.0, sliding: DRY_SLIDING, kx: 1.0, ky: 1.0 };
}

fn mf(b: f64, c: f64, d: f64, e: f64, x: f64) -> f64 {
    let bx = b * x;
    d * (c * (bx - e * (bx - bx.atan())).atan()).sin()
}

/// C from the sliding level: the curve tends to D sin(C pi / 2).
fn shape_c(sliding: f64) -> f64 {
    2.0 - sliding.clamp(0.0, 1.0).asin() * (2.0 / std::f64::consts::PI)
}

/// E that puts the peak at `xm` given the stiffness-to-peak ratio `k_over_d`
/// and C (from B xm - E (B xm - atan(B xm)) = tan(pi / 2C)).
fn shape_e(k_over_d: f64, c: f64, xm: f64) -> f64 {
    let bxm = k_over_d / c * xm;
    (bxm - (std::f64::consts::FRAC_PI_2 / c).tan()) / (bxm - bxm.atan())
}

/// Peak friction and slip stiffness per unit load at load `fz`, from the
/// property file (coefficients evaluated within its FZMIN..FZMAX).
fn longitudinal(fz: f64) -> (f64, f64) {
    let dfz = (fz.clamp(FZMIN, FZMAX) - FNOMIN) / FNOMIN;
    (PDX1 + PDX2 * dfz, (PKX1 + PKX2 * dfz) * (PKX3 * dfz).exp())
}

fn lateral(fz: f64) -> (f64, f64) {
    let fzc = fz.clamp(FZMIN, FZMAX);
    let dfz = (fzc - FNOMIN) / FNOMIN;
    (PDY1 + PDY2 * dfz, -PKY1 * FNOMIN * (2.0 * (fzc / (PKY2 * FNOMIN)).atan()).sin() / fzc)
}

/// The curvature E of each direction, fixed so dry asphalt at 3000 N peaks
/// where the measured tyre does.
fn curvatures() -> (f64, f64) {
    let c = shape_c(DRY_SLIDING);
    let (mx, kx) = longitudinal(REF_LOAD);
    let (my, ky) = lateral(REF_LOAD);
    (shape_e(kx / mx, c, KAPPA_PEAK), shape_e(ky / my, c, ALPHA_PEAK))
}

/// Slip at the peak of sin(C atan(Bx - E(Bx - atan Bx))), by Newton.
fn peak_slip(b: f64, c: f64, e: f64) -> f64 {
    let target = (std::f64::consts::FRAC_PI_2 / c).tan();
    let mut u = target; // u = B x
    for _ in 0..8 {
        let f = u - e * (u - u.atan()) - target;
        let df = 1.0 - e * (1.0 - 1.0 / (1.0 + u * u));
        u -= f / df;
    }
    u / b
}

/// Longitudinal and lateral force (N) for slip ratio `kappa`, slip angle
/// `alpha` (radians, |alpha| <= pi/2) and load `fz` (N). Positive `alpha`
/// (contact patch moving to the wheel's right) gives a positive lateral
/// force here; the caller applies it against the slip.
pub fn force(kappa: f64, alpha: f64, fz: f64, s: Scale) -> (f64, f64) {
    if fz <= 0.0 {
        return (0.0, 0.0);
    }
    let (ex, ey) = curvatures();
    let c = shape_c(s.sliding);
    let (mux, kx) = longitudinal(fz);
    let (muy, ky) = lateral(fz);
    let dx = mux * s.mu * fz;
    let dy = muy * s.mu * fz;
    let bx = kx * s.kx * fz / (c * dx);
    let by = ky * s.ky * fz / (c * dy);
    let km = peak_slip(bx, c, ex);
    let am = peak_slip(by, c, ey);
    let (kn, an) = (kappa / km, alpha / am);
    let n = kn.hypot(an);
    if n < 1e-12 {
        return (0.0, 0.0);
    }
    (kn / n * mf(bx, c, dx, ex, n * km), an / n * mf(by, c, dy, ey, n * am))
}

/// Pneumatic trail (m): how far behind the contact centre the lateral force
/// acts, so the aligning torque is -trail x Fy. MF 5.2 form
/// t = Dt cos(Ct atan(Bt a - Et (Bt a - atan Bt a))) cos a with the file's
/// nominal-load coefficients (its load and camber terms are not quoted):
/// 19 mm at 3000 N near zero slip, falling through zero near 11 degrees.
pub fn trail(alpha: f64, fz: f64) -> f64 {
    let fz = fz.clamp(0.0, FZMAX);
    let dt = fz * QDZ1 * R0 / FNOMIN;
    let ba = QBZ1 * alpha.abs();
    dt * (QCZ1 * (ba - QEZ1 * (ba - ba.atan())).atan()).cos() * alpha.cos()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(f: impl Fn(f64) -> f64, hi: f64) -> (f64, f64) {
        (0..=20000).map(|i| hi * i as f64 / 20000.0).map(|x| (x, f(x))).fold((0.0, 0.0), |a, p| if p.1 > a.1 { p } else { a })
    }

    /// The characteristic values the module doc states.
    #[test]
    fn dry_tyre_matches_the_measured_characteristic_values() {
        let fz = 3000.0;
        let dry = Scale::DRY;
        let (k, fx) = peak(|k| force(k, 0.0, fz, dry).0, 1.0);
        assert!((fx / fz - 1.214).abs() < 0.002 && (k - KAPPA_PEAK).abs() < 0.002, "{k} {}", fx / fz);
        let (a, fy) = peak(|a| force(0.0, a, fz, dry).1, 1.5);
        assert!((fy / fz - 1.1185).abs() < 0.002 && (a - ALPHA_PEAK).abs() < 0.002, "{a} {}", fy / fz);
        assert!((force(0.0, 1e-6, fz, dry).1 / 1e-6 / fz - 19.22).abs() < 0.05);
        assert!((force(1e-6, 0.0, fz, dry).0 / 1e-6 / fz - 27.57).abs() < 0.05);
        // Sliding: locked, and sideways at 90 degrees, near Wong's 0.88 of the peak.
        let locked = force(-1.0, 0.0, fz, dry).0.abs() / fx;
        let side = force(0.0, std::f64::consts::FRAC_PI_2, fz, dry).1 / fy;
        assert!((0.88..0.95).contains(&locked) && (0.88..0.95).contains(&side), "{locked} {side}");
    }

    #[test]
    fn trail_shrinks_with_slip_and_turns_negative() {
        assert!((trail(0.0, 3000.0) - 0.0188).abs() < 0.0005);
        assert!(trail(0.05, 3000.0) < trail(0.0, 3000.0) && trail(0.05, 3000.0) > 0.0);
        assert!(trail(0.3, 3000.0) < 0.0 && trail(0.3, 3000.0) > -0.01);
    }

    #[test]
    fn lower_sliding_level_drops_more_past_the_peak_and_keeps_stiffness() {
        let wet_earth = Scale { mu: 0.55 / 0.85, sliding: 0.45 / 0.55, ..Scale::DRY };
        let fz = 3000.0;
        assert!((force(0.0, 1e-6, fz, wet_earth).1 / force(0.0, 1e-6, fz, Scale::DRY).1 - 1.0).abs() < 1e-3);
        let (_, p) = peak(|a| force(0.0, a, fz, wet_earth).1, 1.5);
        assert!((p / fz - 1.1185 * 0.55 / 0.85).abs() < 0.002);
        let side = force(0.0, 1.5, fz, wet_earth).1 / p;
        assert!(side < 0.86, "{side}");
    }

    #[test]
    fn combined_slip_shares_the_friction_and_stays_odd() {
        for fz in [500.0, 3000.0, 6000.0, 12000.0] {
            for k in [-1.0, -0.3, -0.1, 0.0, 0.05, 0.2, 1.0, 4.0] {
                for a in [-1.5, -0.4, -0.1, 0.0, 0.03, 0.2, 0.8, 1.5] {
                    let (fx, fy) = force(k, a, fz, Scale::DRY);
                    assert!(fx.is_finite() && fy.is_finite());
                    // Never above the longitudinal peak (1.247 at the lightest load).
                    assert!(fx.hypot(fy) <= 1.247 * fz, "{fz} {k} {a}: {fx} {fy}");
                    let (mx, my) = force(-k, -a, fz, Scale::DRY);
                    assert!((fx + mx).abs() < 1e-9 * fz && (fy + my).abs() < 1e-9 * fz, "odd {k} {a}");
                    assert!(fx * k >= 0.0 && fy * a >= 0.0, "{fz} {k} {a}: {fx} {fy}");
                }
            }
        }
        // Braking at the peak slip leaves little cornering force.
        let pure = force(0.0, 0.1, 3000.0, Scale::DRY).1;
        assert!(force(-0.144, 0.1, 3000.0, Scale::DRY).1 < 0.6 * pure);
    }
}
