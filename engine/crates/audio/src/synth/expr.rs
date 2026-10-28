//! Per-sample math for patches: `f1 + (f0 - f1) * exp(-t * k)`. Formulas
//! from procedural sound code (GDScript, C) port as they are. Compiled once
//! (when a voice is built) to a small stack program.
//!
//! Variables: `t` (seconds since the voice started), `sr`, and up to six
//! named inputs. Operators `+ - * / ^`, unary `-`, comparisons `< >` (1 or
//! 0), and functions exp, log, sin, cos, tan, tanh, sqrt, abs, sign, floor,
//! fract, min, max, pow, clamp, lerp, smoothstep, step.

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op { Num(f32), T, Sr, Var(u8), Neg, Add, Sub, Mul, Div, Pow, Lt, Gt, F1(fn(f32) -> f32), F2(fn(f32, f32) -> f32), F3(fn(f32, f32, f32) -> f32) }

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
                Op::F1(f) => st[n - 1] = f(st[n - 1]),
                Op::F3(f) => { n -= 2; st[n - 1] = f(st[n - 1], st[n], st[n + 1]); }
                op => {
                    n -= 1;
                    let (a, b) = (st[n - 1], st[n]);
                    st[n - 1] = match op {
                        Op::Add => a + b, Op::Sub => a - b, Op::Mul => a * b, Op::Div => if b != 0.0 { a / b } else { 0.0 },
                        Op::Pow => a.max(0.0).powf(b), Op::Lt => (a < b) as u8 as f32, Op::Gt => (a > b) as u8 as f32,
                        Op::F2(f) => f(a, b),
                        _ => 0.0,
                    };
                }
            }
        }
        if n == 0 { 0.0 } else { st[n - 1] }
    }

    pub fn fits(&self) -> bool { self.depth <= 32 }
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
        ("exp", 1) => Op::F1(f32::exp), ("log", 1) => Op::F1(|x| x.max(1e-30).ln()), ("sin", 1) => Op::F1(f32::sin), ("cos", 1) => Op::F1(f32::cos),
        ("tan", 1) => Op::F1(f32::tan), ("tanh", 1) => Op::F1(f32::tanh), ("sqrt", 1) => Op::F1(|x| x.max(0.0).sqrt()), ("abs", 1) => Op::F1(f32::abs),
        ("sign", 1) => Op::F1(|x| if x > 0.0 { 1.0 } else if x < 0.0 { -1.0 } else { 0.0 }), ("floor", 1) => Op::F1(f32::floor), ("fract", 1) => Op::F1(|x| x - x.floor()),
        ("min", 2) => Op::F2(f32::min), ("max", 2) => Op::F2(f32::max), ("pow", 2) => Op::F2(|a, b| a.max(0.0).powf(b)), ("step", 2) => Op::F2(|e, x| (x >= e) as u8 as f32),
        ("clamp", 3) => Op::F3(|x, a, b| x.clamp(a.min(b), b.max(a))), ("lerp", 3) => Op::F3(|a, b, t| a + (b - a) * t),
        ("smoothstep", 3) => Op::F3(|a, b, x| { let u = ((x - a) / (b - a)).clamp(0.0, 1.0); u * u * (3.0 - 2.0 * u) }),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(Expr::compile("nope + 1", &[]).unwrap_err().contains("unknown variable"));
        assert!(Expr::compile("lerp(1, 2)", &[]).is_err());
    }
}
