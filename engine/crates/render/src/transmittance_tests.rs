//! CPU interval oracles using the production WGSL step expression. These
//! validate integration extent, not canopy texture fidelity or GPU images.
use naga::{BinaryOperator, Expression, Function, Handle, Literal, MathFunction, Statement};

struct March {
    module: naga::Module,
    function: Handle<Function>,
    step: Handle<Expression>,
    start: f32,
    count: usize,
}
impl March {
    fn production() -> Self {
        let source = include_str!("shaders/shade.wgsl");
        let start = source.find("fn transmittance(").unwrap();
        let mut depth = 0;
        let mut end = None;
        for (offset, c) in source[start..].char_indices() {
            if c == '{' {
                depth += 1;
            }
            if c == '}' {
                depth -= 1;
                if depth == 0 {
                    end = Some(start + offset + 1);
                    break;
                }
            }
        }
        let text = format!(
            "fn ground_at(p:vec2<f32>)->f32{{return -1e9;}}\nfn foliage(p:vec3<f32>)->f32{{return 0.0;}}\n{}",
            &source[start..end.unwrap()]
        );
        let module = naga::front::wgsl::parse_str(&text)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(&text)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap();
        let (function, f) = module
            .functions
            .iter()
            .find(|(_, f)| f.name.as_deref() == Some("transmittance"))
            .unwrap();
        let step = *f
            .named_expressions
            .iter()
            .find(|(_, name)| name.as_str() == "step")
            .unwrap()
            .0;
        let t = f
            .local_variables
            .iter()
            .find(|(_, v)| v.name.as_deref() == Some("t"))
            .unwrap()
            .1;
        let initial = scalar(f, t.init.unwrap(), 0.0, 0.0);
        let count = f
            .expressions
            .iter()
            .find_map(|(_, e)| match *e {
                Expression::Binary {
                    op: BinaryOperator::Less,
                    left,
                    right,
                } if is_local(f, left, "k") => Some(scalar(f, right, 0.0, 0.0) as usize),
                _ => None,
            })
            .expect("bounded production for-loop");
        // The range guard must execute before any terrain/foliage fetch. This
        // catches a zero-length request still sampling an occluding surface.
        let body = f
            .body
            .iter()
            .find_map(|s| {
                if let Statement::Loop { body, .. } = s {
                    Some(body)
                } else {
                    None
                }
            })
            .unwrap();
        let mut guarded = false;
        let mut statements = Vec::new();
        linear_blocks(body, &mut statements);
        for s in statements {
            if let Statement::If {
                condition, accept, ..
            } = s
            {
                if matches!(f.expressions[*condition],Expression::Binary {op:BinaryOperator::GreaterEqual,left,right} if is_local(f,left,"t") && matches!(f.expressions[right],Expression::FunctionArgument(2)))
                {
                    guarded = accept.iter().any(|s| matches!(s, Statement::Break));
                }
            }
            if matches!(s, Statement::Call { .. }) {
                assert!(guarded, "range guard must precede field reads");
                break;
            }
        }
        assert!(guarded);
        Self {
            module,
            function,
            step,
            start: initial,
            count,
        }
    }
    fn intervals(&self, end: f32) -> Vec<(f32, f32)> {
        let mut intervals = Vec::new();
        let mut t = self.start;
        for _ in 0..self.count {
            if t >= end {
                break;
            }
            let length = scalar(&self.module.functions[self.function], self.step, t, end);
            assert!(length > 0.0 && length.is_finite());
            intervals.push((t, length));
            t += length;
        }
        intervals
    }
}
fn linear_blocks<'a>(body: &'a naga::Block, output: &mut Vec<&'a Statement>) {
    for s in body.iter() {
        if let Statement::Block(nested) = s {
            linear_blocks(nested, output);
        } else {
            output.push(s);
        }
    }
}
fn is_local(f: &Function, h: Handle<Expression>, name: &str) -> bool {
    if let Expression::Load { pointer } = f.expressions[h] {
        if let Expression::LocalVariable(v) = f.expressions[pointer] {
            return f.local_variables[v].name.as_deref() == Some(name);
        }
    }
    false
}
fn scalar(f: &Function, h: Handle<Expression>, t: f32, end: f32) -> f32 {
    let eval = |h| scalar(f, h, t, end);
    match f.expressions[h] {
        Expression::Literal(Literal::F32(x)) => x,
        Expression::Literal(Literal::I32(x)) => x as f32,
        Expression::FunctionArgument(2) => end,
        Expression::Load { .. } if is_local(f, h, "t") => t,
        Expression::Binary { op, left, right } => match op {
            BinaryOperator::Multiply => eval(left) * eval(right),
            BinaryOperator::Subtract => eval(left) - eval(right),
            _ => panic!("unexpected interval operator {op:?}"),
        },
        Expression::Math {
            fun,
            arg,
            arg1: Some(b),
            ..
        } => match fun {
            MathFunction::Max => eval(arg).max(eval(b)),
            MathFunction::Min => eval(arg).min(eval(b)),
            _ => panic!("unexpected interval function {fun:?}"),
        },
        ref e => panic!("unexpected interval expression {e:?}"),
    }
}

#[test]
fn requested_interval_is_never_overshot_or_sampled_when_empty() {
    let m = March::production();
    for end in [
        -1.0, 0.0, 0.25, 0.5, 0.501, 0.9, 1.1, 2.0, 10.0, 40.0, 50.0, 60.0,
    ] {
        let intervals = m.intervals(end);
        let length: f32 = intervals.iter().map(|(_, ds)| ds).sum();
        assert!(
            (length - (end - m.start).max(0.0)).abs() < 1e-5,
            "end{end}, length{length}"
        );
        for (t, ds) in intervals {
            assert!(t < end && t + ds <= end + 1e-5);
        }
    }
}

#[test]
fn constant_density_matches_analytic_beer_lambert_on_requested_length() {
    let m = March::production();
    // sigma includes the unchanged0.9 extinction coefficient. Low optical
    // depths avoid the deliberately unchanged transmittance early-out0.02.
    for end in [0.5, 0.6, 2.0, 17.5, 40.0, 50.0, 60.0] {
        for sigma in [0.0f32, 0.0045, 0.045] {
            let actual = m
                .intervals(end)
                .iter()
                .fold(1.0, |tr, (_, ds)| tr * (-sigma * ds).exp());
            let expected = (-sigma * (end - m.start).max(0.0)).exp();
            assert!(
                (actual - expected).abs() < 3e-7,
                "{end}, {sigma}, {actual}, {expected}"
            );
        }
    }
}

#[test]
fn partial_last_segment_preserves_existing_positions_and_unmarched_tail_is_explicit() {
    let m = March::production();
    let short = m.intervals(50.0);
    let unbounded = m.intervals(1000.0);
    assert_eq!(m.count, 12, "cost budget is part of this bounded fix");
    assert_eq!(short.len(), unbounded.len());
    for (a, b) in short.iter().zip(&unbounded) {
        assert_eq!(a.0.to_bits(), b.0.to_bits());
    }
    let last = short.last().unwrap();
    assert!(last.1 < 2.0, "last48m sample must not receive a21m weight");
    assert!((last.0 + last.1 - 50.0).abs() < 1e-5);
    let certified_end = unbounded.last().map(|(t, ds)| t + ds).unwrap();
    assert!(certified_end > 69.0 && certified_end < 70.0);
    assert!(
        certified_end < 1000.0,
        "12 samples do not prove visibility beyond their integration extent"
    );
}
