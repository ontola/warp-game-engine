//! Per-sample math for patches: `f1 + (f0 - f1) * exp(-t * k)`. Formulas
//! from procedural sound code (GDScript, C) port as they are. Compiled once
//! (when a voice is built) to a small stack program.
//!
//! Variables: `t` (seconds since the voice started), `sr`, and up to six
//! named inputs. Operators `+ - * / ^`, unary `-`, comparisons `< >` (1 or
//! 0), and functions exp, log, sin, cos, tan, tanh, sqrt, abs, sign, floor,
//! fract, min, max, pow, fmod (mod), clamp, lerp, smoothstep, step.

use super::BLOCK;
use super::units::Sig;

/// One stack entry for block evaluation: a constant, or a block of samples.
#[derive(Clone)]
pub struct Slot { k: Option<f32>, b: [f32; BLOCK] }

#[inline(always)]
fn bin(a: &mut Slot, b: &Slot, n: usize, f: impl Fn(f32, f32) -> f32) {
    match (a.k, b.k) {
        (Some(x), Some(y)) => a.k = Some(f(x, y)),
        (Some(x), None) => { for i in 0..n { a.b[i] = f(x, b.b[i]); } a.k = None; }
        (None, Some(y)) => for v in &mut a.b[..n] { *v = f(*v, y); },
        (None, None) => for i in 0..n { a.b[i] = f(a.b[i], b.b[i]); },
    }
}

#[inline(always)]
fn un(a: &mut Slot, n: usize, f: impl Fn(f32) -> f32) {
    match a.k { Some(x) => a.k = Some(f(x)), None => for v in &mut a.b[..n] { *v = f(*v); } }
}

/// exp to about 2e-7 relative, branch-free (vectorises).
#[inline(always)]
pub fn fast_exp(x: f32) -> f32 {
    let y = x.clamp(-87.0, 87.0) * std::f32::consts::LOG2_E;
    let i = y.floor();
    let f = y - i;
    // 2^f on [0, 1).
    let p = 1.0 + f * (0.693_147_2 + f * (0.240_226_5 + f * (0.055_504_11 + f * (0.009_618_129 + f * (0.001_333_355 + f * 0.000_154_035_3)))));
    f32::from_bits(p.to_bits().wrapping_add(((i as i32) << 23) as u32))
}

/// ln for x > 0, to about 1e-6.
#[inline(always)]
pub fn fast_ln(x: f32) -> f32 {
    let x = x.max(1e-30);
    let bits = x.to_bits();
    let e = ((bits >> 23) & 0xff) as i32 - 127;
    let m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000); // [1, 2)
    // ln(m) via atanh series on (m - 1) / (m + 1).
    let z = (m - 1.0) / (m + 1.0);
    let z2 = z * z;
    let l = 2.0 * z * (1.0 + z2 * (1.0 / 3.0 + z2 * (1.0 / 5.0 + z2 * (1.0 / 7.0 + z2 * (1.0 / 9.0)))));
    l + e as f32 * std::f32::consts::LN_2
}

/// sin(2π p) for any p, to about 1e-6, branch-free.
#[inline(always)]
pub fn sin_turns(p: f32) -> f32 {
    // Fold to [-0.25, 0.25] turns, then an odd polynomial.
    let q = p - (p + 0.5).floor(); // [-0.5, 0.5)
    let r = if q > 0.25 { 0.5 - q } else if q < -0.25 { -0.5 - q } else { q };
    let x = r * std::f32::consts::TAU;
    let x2 = x * x;
    x * (1.0 + x2 * (-1.0 / 6.0 + x2 * (1.0 / 120.0 + x2 * (-1.0 / 5040.0 + x2 * (1.0 / 362_880.0 - x2 / 39_916_800.0)))))
}

#[inline(always)]
pub fn fast_sin(x: f32) -> f32 { sin_turns(x * (1.0 / std::f32::consts::TAU)) }

#[inline(always)]
pub fn fast_tanh(x: f32) -> f32 {
    let e = fast_exp(2.0 * x.clamp(-9.0, 9.0));
    (e - 1.0) / (e + 1.0)
}

#[inline(always)]
fn fast_pow(a: f32, b: f32) -> f32 {
    if a <= 0.0 { return if b == 0.0 { 1.0 } else { 0.0 }; }
    fast_exp(b * fast_ln(a))
}

#[derive(Clone, Copy, Debug)]
enum F1 { Exp, Ln, Sin, Cos, Tan, Tanh, Sqrt, Abs, Sign, Floor, Fract }
#[derive(Clone, Copy, Debug)]
enum F2 { Min, Max, Pow, Step, Mod }
#[derive(Clone, Copy, Debug)]
enum F3 { Clamp, Lerp, Smooth }

// Each function as a direct (inlinable) closure, so block loops vectorise.
macro_rules! with_f1 { ($f:expr, |$g:ident| $body:expr) => { match $f {
    F1::Exp => { let $g = fast_exp; $body } F1::Ln => { let $g = fast_ln; $body } F1::Sin => { let $g = fast_sin; $body }
    F1::Cos => { let $g = |x: f32| sin_turns(x * (1.0 / std::f32::consts::TAU) + 0.25); $body } F1::Tan => { let $g = f32::tan; $body } F1::Tanh => { let $g = fast_tanh; $body }
    F1::Sqrt => { let $g = |x: f32| x.max(0.0).sqrt(); $body } F1::Abs => { let $g = f32::abs; $body }
    F1::Sign => { let $g = |x: f32| if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 }; $body }
    F1::Floor => { let $g = f32::floor; $body } F1::Fract => { let $g = |x: f32| x - x.floor(); $body }
} } }
macro_rules! with_f2 { ($f:expr, |$g:ident| $body:expr) => { match $f {
    F2::Min => { let $g = f32::min; $body } F2::Max => { let $g = f32::max; $body } F2::Pow => { let $g = fast_pow; $body }
    F2::Step => { let $g = |e: f32, x: f32| (x >= e) as u8 as f32; $body }
    F2::Mod => { let $g = |a: f32, b: f32| if b != 0.0 { a - b * (a / b).floor() } else { 0.0 }; $body }
} } }
macro_rules! with_f3 { ($f:expr, |$g:ident| $body:expr) => { match $f {
    F3::Clamp => { let $g = |x: f32, a: f32, b: f32| x.clamp(a.min(b), b.max(a)); $body }
    F3::Lerp => { let $g = |a: f32, b: f32, t: f32| a + (b - a) * t; $body }
    F3::Smooth => { let $g = |a: f32, b: f32, x: f32| { let u = ((x - a) / (b - a)).clamp(0.0, 1.0); u * u * (3.0 - 2.0 * u) }; $body }
} } }

#[derive(Clone, Copy, Debug)]
enum Op { Num(f32), T, Sr, Var(u8), Neg, Add, Sub, Mul, Div, Pow, Lt, Gt, F1(F1), F2(F2), F3(F3) }

#[derive(Clone, Debug)]
pub struct Expr { ops: Vec<Op>, depth: usize }

impl Expr {
    /// Compile `src` with input variables named `vars` (in input order).
    pub fn compile(src: &str, vars: &[String]) -> Result<Expr, String> {
        let toks = lex(src)?;
        let mut p = Parser { toks, k: 0, vars, ops: Vec::new() };
        p.expr()?;
        if p.k != p.toks.len() { return Err(format!("unexpected {:?} in `{src}`", p.toks[p.k])); }
        let mut depth: i32 = 0;
        let mut max = 0;
        for op in &p.ops {
            depth += match op { Op::Num(_) | Op::T | Op::Sr | Op::Var(_) => 1, Op::Neg | Op::F1(_) => 0, Op::F3(_) => -2, _ => -1 };
            max = max.max(depth);
        }
        Ok(Expr { ops: p.ops, depth: max as usize })
    }

    #[inline]
    pub fn eval(&self, t: f32, sr: f32, vars: &[f32; 6]) -> f32 {
        let mut st = [0.0f32; 32];
        let mut n = 0usize;
        for op in &self.ops {
            match *op {
                Op::Num(v) => { st[n] = v; n += 1; }
                Op::T => { st[n] = t; n += 1; }
                Op::Sr => { st[n] = sr; n += 1; }
                Op::Var(k) => { st[n] = vars[k as usize]; n += 1; }
                Op::Neg => st[n - 1] = -st[n - 1],
                Op::F1(f) => st[n - 1] = with_f1!(f, |g| g(st[n - 1])),
                Op::F3(f) => { n -= 2; st[n - 1] = with_f3!(f, |g| g(st[n - 1], st[n], st[n + 1])); }
                op => {
                    n -= 1;
                    let (a, b) = (st[n - 1], st[n]);
                    st[n - 1] = match op {
                        Op::Add => a + b, Op::Sub => a - b, Op::Mul => a * b, Op::Div => if b != 0.0 { a / b } else { 0.0 },
                        Op::Pow => fast_pow(a, b), Op::Lt => (a < b) as u8 as f32, Op::Gt => (a > b) as u8 as f32,
                        Op::F2(f) => with_f2!(f, |g| g(a, b)),
                        _ => 0.0,
                    };
                }
            }
        }
        if n == 0 { 0.0 } else { st[n - 1] }
    }

    /// Stack space for [`Expr::eval_block`].
    pub fn slots(&self) -> Vec<Slot> { vec![Slot { k: Some(0.0), b: [0.0; BLOCK] }; self.depth.max(1)] }

    /// Evaluate over a block: one operation at a time across all samples,
    /// with constants kept as scalars (so constant parts cost one sample).
    pub fn eval_block(&self, t0: f32, dt: f32, sr: f32, vars: &[Sig; 6], st: &mut [Slot], out: &mut [f32]) {
        let n = out.len();
        let mut sp = 0usize;
        for op in &self.ops {
            match *op {
                Op::Num(v) => { st[sp].k = Some(v); sp += 1; }
                Op::Sr => { st[sp].k = Some(sr); sp += 1; }
                Op::T => { let s = &mut st[sp]; s.k = None; for (i, v) in s.b[..n].iter_mut().enumerate() { *v = t0 + i as f32 * dt; } sp += 1; }
                Op::Var(k) => {
                    let s = &mut st[sp];
                    match vars[k as usize] {
                        Sig::K(v) => s.k = Some(v),
                        Sig::B(b) => { s.k = None; s.b[..n].copy_from_slice(&b[..n]); }
                        r => { s.k = None; for (i, v) in s.b[..n].iter_mut().enumerate() { *v = r.at(i); } }
                    }
                    sp += 1;
                }
                Op::Neg => un(&mut st[sp - 1], n, |x| -x),
                Op::F1(f) => with_f1!(f, |g| un(&mut st[sp - 1], n, g)),
                Op::F3(f) => {
                    sp -= 2;
                    let (lo, hi) = st.split_at_mut(sp);
                    let (a, rest) = (&mut lo[sp - 1], hi);
                    let (b, c) = (&rest[0], &rest[1]);
                    if let (Some(x), Some(y), Some(z)) = (a.k, b.k, c.k) { a.k = Some(with_f3!(f, |g| g(x, y, z))); continue; }
                    // Constants spread to blocks (rare enough).
                    let bb: [f32; BLOCK] = match b.k { Some(v) => [v; BLOCK], None => b.b };
                    let cb: [f32; BLOCK] = match c.k { Some(v) => [v; BLOCK], None => c.b };
                    if let Some(v) = a.k { a.b = [v; BLOCK]; }
                    with_f3!(f, |g| for i in 0..n { a.b[i] = g(a.b[i], bb[i], cb[i]); });
                    a.k = None;
                }
                op => {
                    sp -= 1;
                    let (lo, hi) = st.split_at_mut(sp);
                    let (a, b) = (&mut lo[sp - 1], &hi[0]);
                    match op {
                        Op::Add => bin(a, b, n, |x, y| x + y),
                        Op::Sub => bin(a, b, n, |x, y| x - y),
                        Op::Mul => bin(a, b, n, |x, y| x * y),
                        Op::Div => bin(a, b, n, |x, y| if y != 0.0 { x / y } else { 0.0 }),
                        Op::Pow => bin(a, b, n, fast_pow),
                        Op::Lt => bin(a, b, n, |x, y| (x < y) as u8 as f32),
                        Op::Gt => bin(a, b, n, |x, y| (x > y) as u8 as f32),
                        Op::F2(f) => with_f2!(f, |g| bin(a, b, n, g)),
                        _ => {}
                    }
                }
            }
        }
        match sp.checked_sub(1).map(|k| &st[k]) {
            Some(Slot { k: Some(v), .. }) => out.fill(*v),
            Some(s) => out.copy_from_slice(&s.b[..n]),
            None => out.fill(0.0),
        }
    }

    pub fn fits(&self) -> bool { self.depth <= 32 }
    /// Whether the value changes with time on its own (reads `t`).
    pub fn uses_t(&self) -> bool { self.ops.iter().any(|o| matches!(o, Op::T)) }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok { Num(f32), Id(String), Sym(char) }

fn lex(s: &str) -> Result<Vec<Tok>, String> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut k = 0;
    while k < c.len() {
        let ch = c[k];
        if ch.is_whitespace() { k += 1; continue; }
        if ch.is_ascii_digit() || (ch == '.' && c.get(k + 1).is_some_and(|d| d.is_ascii_digit())) {
            let st = k;
            while k < c.len() && (c[k].is_ascii_digit() || c[k] == '.' || ((c[k] == 'e' || c[k] == 'E') && k + 1 < c.len() && (c[k + 1].is_ascii_digit() || c[k + 1] == '-'))) {
                if c[k] == 'e' || c[k] == 'E' { k += 1; }
                k += 1;
            }
            let txt: String = c[st..k].iter().collect();
            out.push(Tok::Num(txt.parse().map_err(|_| format!("bad number {txt}"))?));
            continue;
        }
        if ch.is_alphabetic() || ch == '_' {
            let st = k;
            while k < c.len() && (c[k].is_alphanumeric() || c[k] == '_') { k += 1; }
            out.push(Tok::Id(c[st..k].iter().collect()));
            continue;
        }
        if "+-*/^(),<>".contains(ch) { out.push(Tok::Sym(ch)); k += 1; continue; }
        return Err(format!("unexpected '{ch}' in `{s}`"));
    }
    Ok(out)
}

struct Parser<'a> { toks: Vec<Tok>, k: usize, vars: &'a [String], ops: Vec<Op> }

impl Parser<'_> {
    fn peek(&self) -> Option<&Tok> { self.toks.get(self.k) }
    fn eat(&mut self, c: char) -> bool { if self.peek() == Some(&Tok::Sym(c)) { self.k += 1; true } else { false } }

    fn expr(&mut self) -> Result<(), String> {
        self.sum()?;
        loop {
            if self.eat('<') { self.sum()?; self.ops.push(Op::Lt); } else if self.eat('>') { self.sum()?; self.ops.push(Op::Gt); } else { return Ok(()); }
        }
    }
    fn sum(&mut self) -> Result<(), String> {
        self.product()?;
        loop {
            if self.eat('+') { self.product()?; self.ops.push(Op::Add); } else if self.eat('-') { self.product()?; self.ops.push(Op::Sub); } else { return Ok(()); }
        }
    }
    fn product(&mut self) -> Result<(), String> {
        self.unary()?;
        loop {
            if self.eat('*') { self.unary()?; self.ops.push(Op::Mul); } else if self.eat('/') { self.unary()?; self.ops.push(Op::Div); } else { return Ok(()); }
        }
    }
    fn unary(&mut self) -> Result<(), String> {
        if self.eat('-') { self.unary()?; self.ops.push(Op::Neg); return Ok(()); }
        self.power()
    }
    fn power(&mut self) -> Result<(), String> {
        self.atom()?;
        if self.eat('^') { self.unary()?; self.ops.push(Op::Pow); }
        Ok(())
    }
    fn atom(&mut self) -> Result<(), String> {
        match self.toks.get(self.k).cloned() {
            Some(Tok::Num(v)) => { self.k += 1; self.ops.push(Op::Num(v)); Ok(()) }
            Some(Tok::Sym('(')) => { self.k += 1; self.expr()?; if !self.eat(')') { return Err("missing )".into()); } Ok(()) }
            Some(Tok::Id(name)) => {
                self.k += 1;
                if self.eat('(') {
                    let mut n = 0;
                    if !self.eat(')') {
                        loop { self.expr()?; n += 1; if self.eat(')') { break; } if !self.eat(',') { return Err(format!("{name}(: expected , or )")); } }
                    }
                    let op = func(&name, n).ok_or_else(|| format!("unknown function {name} with {n} arguments"))?;
                    self.ops.push(op);
                    return Ok(());
                }
                let op = match name.as_str() {
                    "t" => Op::T, "sr" => Op::Sr, "pi" => Op::Num(std::f32::consts::PI), "tau" => Op::Num(std::f32::consts::TAU),
                    _ => match self.vars.iter().position(|v| *v == name) {
                        Some(k) => Op::Var(k as u8),
                        None => return Err(format!("unknown variable {name} (inputs: {}, t, sr)", self.vars.join(", "))),
                    },
                };
                self.ops.push(op);
                Ok(())
            }
            other => Err(format!("unexpected {other:?}")),
        }
    }
}

fn func(name: &str, n: usize) -> Option<Op> {
    Some(match (name, n) {
        ("exp", 1) => Op::F1(F1::Exp), ("log", 1) => Op::F1(F1::Ln), ("sin", 1) => Op::F1(F1::Sin), ("cos", 1) => Op::F1(F1::Cos),
        ("tan", 1) => Op::F1(F1::Tan), ("tanh", 1) => Op::F1(F1::Tanh), ("sqrt", 1) => Op::F1(F1::Sqrt), ("abs", 1) => Op::F1(F1::Abs),
        ("sign", 1) => Op::F1(F1::Sign), ("floor", 1) => Op::F1(F1::Floor), ("fract", 1) => Op::F1(F1::Fract),
        ("min", 2) => Op::F2(F2::Min), ("max", 2) => Op::F2(F2::Max), ("pow", 2) => Op::F2(F2::Pow), ("step", 2) => Op::F2(F2::Step), ("fmod", 2) | ("mod", 2) => Op::F2(F2::Mod),
        ("clamp", 3) => Op::F3(F3::Clamp), ("lerp", 3) => Op::F3(F3::Lerp), ("smoothstep", 3) => Op::F3(F3::Smooth),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fast_maths_is_close() {
        for k in -2000..2000 {
            let x = k as f32 * 0.01;
            assert!((fast_exp(x) - x.exp()).abs() <= x.exp() * 1e-5, "exp {x}: {} {}", fast_exp(x), x.exp());
            assert!((fast_tanh(x) - x.tanh()).abs() < 1e-5, "tanh {x}");
            let p = (k + 2001) as f32 * 0.05;
            assert!((fast_ln(p) - p.ln()).abs() < 2e-6, "ln {p}");
        }
        for k in -5000..5000 {
            let x = k as f32 * 0.0137;
            assert!((fast_sin(x) - x.sin()).abs() < 2e-5, "sin {x}");
        }
        assert_eq!(fast_pow(0.0, 1.6), 0.0);
        assert!((fast_pow(0.5, 1.6) - 0.5f32.powf(1.6)).abs() < 1e-6);
    }

    #[test]
    fn formulas_evaluate_like_the_maths() {
        let vars = vec!["f0".to_string(), "f1".into(), "k".into()];
        let e = Expr::compile("f1 + (f0 - f1) * exp(-t * k)", &vars).unwrap();
        let v = [220.0, 55.0, 80.0, 0.0, 0.0, 0.0];
        assert_eq!(e.eval(0.0, 48000.0, &v), 220.0);
        assert!((e.eval(0.1, 48000.0, &v) - (55.0 + 165.0 * (-8f32).exp())).abs() < 1e-3);
        let e = Expr::compile("(1 - t / 0.2) ^ 1.6 * -2 + clamp(t, 0, 1) + (t > 0.5)", &[]).unwrap();
        assert!((e.eval(0.1, 48000.0, &[0.0; 6]) - (0.5f32.powf(1.6) * -2.0 + 0.1)).abs() < 1e-5);
        assert_eq!(e.eval(0.6, 48000.0, &[0.0; 6]), 0.0f32.powf(1.6) * -2.0 + 0.6 + 1.0);
        assert_eq!(Expr::compile("fmod(t, 0.07)", &[]).unwrap().eval(0.1, 48000.0, &[0.0; 6]), 0.1f32 - 0.07);
        assert!(Expr::compile("nope + 1", &[]).unwrap_err().contains("unknown variable"));
        assert!(Expr::compile("lerp(1, 2)", &[]).is_err());
    }
}
