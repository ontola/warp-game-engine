//! CPU execution of parsed WGSL fog expressions and independent quadrature.
//! These do not claim GPU image validation or volumetric light occlusion.

use glam::Vec3;
use naga::{BinaryOperator, Expression, Function, Handle, Literal, MathFunction, Statement};

#[derive(Clone, Copy, Debug)]
enum Value { Scalar(f32), Colour(Vec3), Bool(bool) }
impl Value {
    fn scalar(self) -> f32 { if let Self::Scalar(x) = self { x } else { panic!("expected scalar") } }
    fn colour(self) -> Vec3 { if let Self::Colour(x) = self { x } else { panic!("expected colour") } }
    fn boolean(self) -> bool { if let Self::Bool(x) = self { x } else { panic!("expected boolean") } }
}

// Execute the parsed pure fog helpers, rather than handwritten copies of
// their equations. Unsupported shader changes fail explicitly and require
// reviewing this small interpreter's scope.
fn expression(module: &naga::Module, f: &Function, h: Handle<Expression>, args: &[Value]) -> Value {
    let eval = |h| expression(module, f, h, args);
    match f.expressions[h] {
        Expression::FunctionArgument(i) => args[i as usize],
        Expression::Literal(Literal::F32(x)) => Value::Scalar(x),
        Expression::Constant(handle) => match module.global_expressions[module.constants[handle].init] {
            Expression::Literal(Literal::F32(x)) => Value::Scalar(x),
            ref other => panic!("unexpected fog constant {other:?}"),
        },
        Expression::Unary { op: naga::UnaryOperator::Negate, expr } => Value::Scalar(-eval(expr).scalar()),
        Expression::Splat { value, .. } => Value::Colour(Vec3::splat(eval(value).scalar())),
        Expression::Binary { op, left, right } => {
            use BinaryOperator::*;
            match (eval(left), eval(right)) {
                (Value::Scalar(a), Value::Scalar(b)) => match op {
                    Add=>Value::Scalar(a+b), Subtract=>Value::Scalar(a-b), Multiply=>Value::Scalar(a*b), Divide=>Value::Scalar(a/b),
                    Less=>Value::Bool(a<b), LessEqual=>Value::Bool(a<=b), Greater=>Value::Bool(a>b), GreaterEqual=>Value::Bool(a>=b),
                    _=>panic!("unexpected scalar fog operator {op:?}"),
                },
                (Value::Colour(a), Value::Colour(b)) => match op {
                    Add=>Value::Colour(a+b), Subtract=>Value::Colour(a-b), Multiply=>Value::Colour(a*b), Divide=>Value::Colour(a/b),
                    _=>panic!("unexpected colour fog operator {op:?}"),
                },
                (Value::Colour(a), Value::Scalar(b)) if op==Multiply => Value::Colour(a*b),
                (Value::Scalar(a), Value::Colour(b)) if op==Multiply => Value::Colour(b*a),
                pair=>panic!("unexpected mixed fog operator {op:?}: {pair:?}"),
            }
        }
        Expression::Math { fun: MathFunction::Mix, arg, arg1: Some(b), arg2: Some(t), .. } => {
            Value::Colour(eval(arg).colour().lerp(eval(b).colour(), eval(t).scalar()))
        }
        Expression::Math { fun, arg, arg1, arg2, .. } => {
            let a=eval(arg).scalar();
            let b=||eval(arg1.unwrap()).scalar();
            Value::Scalar(match fun {
                MathFunction::Exp=>a.exp(), MathFunction::Abs=>a.abs(), MathFunction::Max=>a.max(b()), MathFunction::Min=>a.min(b()),
                MathFunction::Pow=>a.powf(b()), MathFunction::Clamp=>a.clamp(b(),eval(arg2.unwrap()).scalar()),
                _=>panic!("unexpected fog math {fun:?}"),
            })
        },
        ref other => panic!("unexpected fog expression {other:?}"),
    }
}
fn block(module: &naga::Module, f: &Function, body: &naga::Block, args: &[Value]) -> Option<Value> {
    for statement in body.iter() {
        match statement {
            Statement::Emit(_) => {},
            Statement::If { condition, accept, reject } => {
                let branch = if expression(module, f, *condition, args).boolean() { accept } else { reject };
                if let Some(value) = block(module, f, branch, args) { return Some(value); }
            }
            Statement::Return { value: Some(value) } => return Some(expression(module, f, *value, args)),
            other => panic!("unexpected fog statement {other:?}"),
        }
    }
    None
}
fn module() -> naga::Module {
    let source = crate::wind::shade_source();
    let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
        .validate(&module).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
    module
}
fn function<'a>(module: &'a naga::Module, name: &str) -> (Handle<Function>, &'a Function) {
    module.functions.iter().find(|(_, f)| f.name.as_deref() == Some(name)).unwrap()
}

#[test]
fn full_fog_matches_background_and_partial_fog_keeps_contrast() {
    let module = module(); let (_, blend) = function(&module, "fog_blend");
    let backgrounds = [Vec3::new(1000.0, 2300.0, 6000.0), Vec3::new(7000.0, 5000.0, 2700.0), Vec3::splat(0.05)];
    for background in backgrounds { for source in [Vec3::ZERO, Vec3::splat(10_000.0), Vec3::new(200.0, 3900.0, 500.0)] {
        let run = |amount| block(&module, blend, &blend.body, &[Value::Colour(source), Value::Colour(background), Value::Scalar(amount)]).unwrap().colour();
        assert_eq!(run(1.0), background, "fully obscured geometry must lose its silhouette in every direction");
        assert_eq!(run(0.0), source, "unfogged radiance must stay unchanged");
        for amount in [0.1, 0.5, 0.9, 0.999] {
            let retained = run(amount) - background;
            assert!((retained - (source - background) * (1.0 - amount)).abs().max_element() < 0.002,
                "partial fog must retain proportional object contrast");
        }
    }}
}

fn scalar_function(module:&naga::Module,name:&str,args:&[f32])->f32 {
    let (_, f)=function(module,name);
    let values:Vec<_>=args.iter().copied().map(Value::Scalar).collect();
    block(module,f,&f.body,&values).unwrap().scalar()
}

#[test]
fn finite_fog_column_matches_independent_quadrature_and_segment_composition() {
    let module=module();
    let column=|d,y,v,h|scalar_function(&module,"fog_column",&[d,y,v,h]);
    for h in [220.0,1200.0] {for y in [-200.0,-10.0,0.0,20.0,1500.0] {
        for v in [-1.0,-0.3,-0.0001,0.0,0.0001,0.3,1.0] {for d in [0.0,0.1,50.0,190.0,650.0,1200.0,5000.0] {
            let actual=column(d,y,v,h);
            let n=4096;
            let reference=(0..n).map(|i| {
                let height=(y as f64+v as f64*d as f64*(i as f64+0.5)/n as f64).max(0.0);
                (-height/h as f64).exp()*d as f64/n as f64
            }).sum::<f64>();
            assert!(actual.is_finite()&&actual>=0.0,"invalid column d={d} y={y} v={v} h={h}: {actual}");
            assert!((actual as f64-reference).abs()<0.001+reference*0.0002,
                "column d={d} y={y} v={v} h={h}: shader={actual} quadrature={reference}");
            let a=d*0.37;
            let pieces=column(a,y,v,h)+column(d-a,y+v*a,v,h);
            assert!((pieces-actual).abs()<0.002+actual*0.0003,"ray partition changed optical depth");
        }}
    }}
}

#[test]
fn finite_fog_phase_is_normalised_and_segment_obeys_beer_lambert() {
    let module=module();
    for g in [0.0,0.6,0.8] {
        let n=20000;
        let integral=(0..n).map(|i|scalar_function(&module,"fog_phase",&[-1.0+(i as f32+0.5)*2.0/n as f32,g])
            *4.0*std::f32::consts::PI/n as f32).sum::<f32>();
        assert!((integral-1.0).abs()<0.0001,"phase normalisation g={g}: {integral}");
    }
    let (_, segment)=function(&module,"fog_segment");
    for source in [Vec3::ZERO,Vec3::new(100.0,20.0,1.0),Vec3::splat(1000.0)] {
        let surface=Vec3::new(4.0,8.0,16.0);
        let run=|tau|block(&module,segment,&segment.body,&[Value::Colour(surface),Value::Colour(source),Value::Scalar(tau)]).unwrap().colour();
        assert_eq!(run(0.0),surface);
        assert_eq!(run(1000.0),source);
        let a=run(0.1);let b=run(1.0);
        assert!((a-(surface*(-0.1f32).exp()+source*(1.0-(-0.1f32).exp()))).abs().max_element()<1e-4);
        assert!((b-source).length()<=(a-source).length());
    }
}

#[test]
fn sunset_mist_and_haze_have_no_baked_horizon_step_or_authored_distance_step() {
    let module=module();let (_, segment)=function(&module,"fog_segment");
    // Captured haze/mist densities and a shaded rock. The exact sky image
    // can contain any horizon jump: it is not an input to this finite ray.
    for (density,h,y) in [(0.0012,1200.0,14.0),(0.011,220.0,1.28)] {
        for distance in [49.99,50.0,50.01,189.99,190.0,190.01,649.99,650.0,650.01] {
            let run=|vertical| {
                let optical=scalar_function(&module,"fog_column",&[distance,y,vertical,h])*density;
                block(&module,segment,&segment.body,&[Value::Colour(Vec3::splat(5.0)),Value::Colour(Vec3::new(700.0,220.0,70.0)),Value::Scalar(optical)]).unwrap().colour()
            };
            assert!((run(-1e-5)-run(1e-5)).abs().max_element()<0.01,"false horizon stripe at {distance}m");
        }
    }
    let (background,_)=function(&module,"background_radiance");
    let (column,_)=function(&module,"fog_column");
    let (_,apply)=function(&module,"apply_distance_fog");
    let physical=apply.body.iter().find_map(|statement|match statement {
        Statement::If {accept,..} if calls(accept,column)=>Some(accept),_=>None
    }).expect("physical finite-distance path");
    assert!(!calls(physical,background),"surface mist must not import the distant sky or planet horizon");
    let (_,source)=function(&module,"fog_scattering_source");
    assert!(!source.expressions.iter().any(|(_,e)|matches!(e,Expression::ImageLoad {..}|Expression::ImageSample {..})),
        "local fog source cannot sample the background LUT");
}

fn calls(body: &naga::Block, target: Handle<Function>) -> bool {
    body.iter().any(|statement| match statement {
        Statement::Call { function, .. } => *function == target,
        Statement::If { accept, reject, .. } => calls(accept, target) || calls(reject, target),
        Statement::Block(body) => calls(body, target),
        Statement::Loop { body, continuing, .. } => calls(body, target) || calls(continuing, target),
        Statement::Switch { cases, .. } => cases.iter().any(|case| calls(&case.body, target)),
        _ => false,
    })
}

#[test]
fn opaque_transparent_and_volume_fog_share_visible_background() {
    let sources = [
        (crate::wind::shade_source(), "shade"),
        (crate::transparency::with_refraction(&crate::transparency::shader_source(), true, true), "transparent_color"),
        (crate::grass::shader_source(), "grass_color"),
        (crate::far_grass::shader_source(), "fg_lighting"),
        (crate::voxel_forest::shader_source(), "volume_radiance"),
    ];
    for (source, consumer) in sources {
        let module = naga::front::wgsl::parse_str(&source).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all())
            .validate(&module).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&source)));
        let (background, surface_background) = function(&module, "background_radiance");
        let (visible_background, visible) = function(&module, "visible_background_radiance");
        let (attenuate, _) = function(&module, "attenuate_background");
        let (sky, _) = function(&module, "background_sky");
        let (stars, _) = function(&module, "night_star_radiance");
        // The underlying sky and atmospheric transmission remain shared.
        // Only an empty depth pixel may add stars; surface fog must not
        // paste stars over an opaque mountain or a transparent material.
        for path in [surface_background, visible] {
            assert!(calls(&path.body, sky));
            assert!(calls(&path.body, attenuate));
        }
        assert!(!calls(&surface_background.body, stars));
        assert!(calls(&visible.body, stars));
        let (fog, apply) = function(&module, "apply_distance_fog");
        assert!(calls(&apply.body, background));
        assert!(!calls(&apply.body, visible_background));
        assert!(!calls(&apply.body, stars));
        let opaque = &module.entry_points.iter().find(|entry| entry.name == "shade").unwrap().function;
        let background_branches: Vec<_> = opaque.body.iter().filter_map(|statement| match statement {
            Statement::If {condition,accept,reject} if calls(accept,visible_background) => Some((*condition,accept,reject)),
            _=>None,
        }).collect();
        assert_eq!(background_branches.len(),1,"stars need one explicit empty-depth background branch");
        let (condition,accept,reject)=background_branches[0];
        let Expression::Binary {op:BinaryOperator::LessEqual,left,right}=opaque.expressions[condition] else {
            panic!("visible sky must be gated by depth <= 0");
        };
        assert!(matches!(opaque.expressions[right],Expression::Literal(Literal::F32(v)) if v==0.0));
        let Expression::ImageLoad {image,..}=opaque.expressions[left] else {
            panic!("visible sky depth gate must read actual opaque depth");
        };
        let Expression::GlobalVariable(depth)=opaque.expressions[image] else {
            panic!("visible sky depth must be a bound texture");
        };
        assert_eq!(module.global_variables[depth].name.as_deref(),Some("depth_tex"));
        assert!(accept.iter().any(|s|matches!(s,Statement::Return {value:None})),
            "empty-depth sky must return before material shading");
        assert!(!calls(reject,visible_background));
        assert!(!calls(&opaque.body,stars),"no direct stellar term in opaque material shading");
        let selected = module.entry_points.iter().find(|entry| entry.name == consumer).map(|entry| &entry.function)
            .or_else(|| module.functions.iter().find(|(_, f)| f.name.as_deref() == Some(consumer)).map(|(_, f)| f))
            .unwrap_or_else(|| panic!("missing fog consumer {consumer}"));
        assert!(calls(&selected.body, fog), "{consumer} must use the common directional fog");
    }
}

#[test]
fn smoke_and_surface_share_finite_atmosphere() {
    let surface = module();
    let smoke = naga::front::wgsl::parse_str(crate::fx::shader_source()).unwrap();
    for distance in [0.0, 10.0, 50.0, 190.0, 650.0] {
        for height in [-2.0, 0.0, 20.0, 200.0] {
            for vertical in [-0.8, -0.00001, 0.0, 0.00001, 0.8] {
                let args = [distance, height, vertical, 220.0];
                assert_eq!(scalar_function(&smoke,"fog_column",&args), scalar_function(&surface,"fog_column",&args));
            }
        }
    }
    for g in [0.0, 0.6, 0.8] { for mu in [-1.0, 0.0, 0.9, 1.0] {
        let args = [mu,g];
        assert!((scalar_function(&smoke,"fog_phase",&args)-scalar_function(&surface,"fog_phase",&args)).abs()<0.00001);
    }}
}
