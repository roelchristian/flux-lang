use super::{lowering, validation::CheckedPlan};
use crate::hir::{CoreOp, ExprId, ExprKind, HirProgram, Target};
use crate::hir_eval::{run_with_options, ResourcePolicy, RuntimeOptions};

fn checked(source: &str) -> HirProgram {
    use crate::semantic::{self, Mode, Policy};
    let program = crate::parser::parse(source).unwrap();
    semantic::prepare(&program, Mode::Script, Policy::TypedOnly)
        .unwrap()
        .hir()
        .unwrap()
        .clone()
}

fn find(program: &HirProgram, op: CoreOp) -> ExprId {
    ExprId(
        program
            .arena
            .iter()
            .rposition(|node| matches!(node.kind, ExprKind::Call(Target::Builtin(k), _) if k == op))
            .expect("checked kernel"),
    )
}

fn plan(program: &HirProgram, root: ExprId, limit: usize) -> Option<CheckedPlan> {
    CheckedPlan::validate(program, lowering::build(program, root, limit)?, limit)
}

#[test]
fn admits_same_shape_direct_real_negation_and_boolean_not() {
    let program = checked("x := -: -: -: form [2n] [-0.0r 1r]; y := ~ ~ form [2n] [0b 1b];");
    let negate = find(&program, CoreOp::Negate);
    assert_eq!(plan(&program, negate, 128).unwrap().stages().len(), 3);
    assert!(plan(&program, negate, 2).is_none());
    let not = find(&program, CoreOp::Not);
    assert_eq!(plan(&program, not, 128).unwrap().stages().len(), 2);
}

#[test]
fn excludes_partial_exact_indirect_and_scalar_expressions() {
    for (source, op) in [
        ("x := -: -: form [1n] [1z];", CoreOp::Negate),
        ("x := -: -: 1r;", CoreOp::Negate),
        ("x := ^/ ^/ form [1n] [4r];", CoreOp::Sqrt),
        (
            "n:array[r]->array[r] := &-:; x := -: @ n (form [1n] [0r]);",
            CoreOp::Negate,
        ),
    ] {
        let program = checked(source);
        assert!(plan(&program, find(&program, op), 128).is_none());
    }
}
#[test]
fn fused_unary_chain_does_not_move_work_or_depth_failures() {
    let program = checked("x := -: -: -: form [4n] [-0.0r 1r -2r 0.0r]; !^{ ! print x; };");
    let mut passed = false;
    let mut failed = false;
    let mut compare = |limits: ResourcePolicy| {
        let attempt = |optimize| {
            run_with_options(
                &program,
                RuntimeOptions {
                    resources: limits,
                    optimize,
                },
            )
            .map(|output| output.stdout)
            .map_err(|error| {
                let diag = error.structured().unwrap();
                (diag.primary, diag.category, error.message)
            })
        };
        let generic = attempt(false);
        assert_eq!(attempt(true), generic, "{limits:?}");
        match generic {
            Ok(stdout) => {
                passed = true;
                assert_eq!(stdout, "array [4] [0 -1 2 -0]\n");
            }
            Err(_) => failed = true,
        }
    };
    for max_work in 1..70 {
        compare(ResourcePolicy {
            max_work,
            ..ResourcePolicy::default()
        });
    }
    for max_depth in 1..10 {
        compare(ResourcePolicy {
            max_depth,
            ..ResourcePolicy::default()
        });
    }
    assert!(passed && failed);
}

#[test]
fn validates_real_pipeline_and_literal_boolean_edges_only() {
    use super::ir::Kernel;
    use crate::hir_eval::numeric::TotalRealUnary;
    let program = checked(
        r#"<< "math";
        a := form [1n] [0r];
        x := math.tanh math.atan math.cos math.sin | -: a;
        bits := form [1n] [1b];
        y := || (&& (~ bits) 1b) 0b;
        z := && (~ bits) (/= (# bits) 0n);"#,
    );
    let tanh = find(&program, CoreOp::MathTanh);
    let real = plan(&program, tanh, 128).unwrap();
    assert_eq!(
        real.stages()
            .iter()
            .map(|stage| stage.kernel)
            .collect::<Vec<_>>(),
        [
            TotalRealUnary::Tanh,
            TotalRealUnary::Atan,
            TotalRealUnary::Cos,
            TotalRealUnary::Sin,
            TotalRealUnary::Abs,
            TotalRealUnary::Negate
        ]
        .map(Kernel::Real)
    );
    let or = find(&program, CoreOp::Or);
    let bools = plan(&program, or, 128).unwrap();
    assert_eq!(bools.stages().len(), 3);
    assert!(bools.boolean(false));
    assert!(!bools.boolean(true));
    assert!(plan(&program, find(&program, CoreOp::And), 128).is_none());

    let too_deep = lowering::build(&program, tanh, 128).unwrap();
    assert!(CheckedPlan::validate(&program, too_deep, 5).is_none());
    let mut wrong_edges = lowering::build(&program, tanh, 128).unwrap();
    wrong_edges.stages.swap(1, 2);
    assert!(CheckedPlan::validate(&program, wrong_edges, 128).is_none());
    let mut wrong_leaf = lowering::build(&program, tanh, 128).unwrap();
    wrong_leaf.leaf = tanh;
    assert!(CheckedPlan::validate(&program, wrong_leaf, 128).is_none());
    let mut wrong_literal = lowering::build(&program, or, 128).unwrap();
    if let Kernel::Or(ref mut rhs) = wrong_literal.stages[0].kernel {
        rhs.value = true;
    } else {
        panic!("expected a Boolean literal proof");
    }
    assert!(CheckedPlan::validate(&program, wrong_literal, 128).is_none());
}

#[test]
fn mixed_total_chains_preserve_strided_values_and_work_depth_faults() {
    let program = checked(
        r#"<< "math";
        << "arr";
        raw := form [3n 2n] [-0.0r -1r 1.7976931348623157e308r 2r -5e-324r 3r];
        real := |: (arr.sect [1n 1n] [3n 2n] raw);
        flags := |: (form [3n 2n] [0b 1b 1b 0b 0b 1b]);
        !^{
            ! print ,. (math.tanh math.atan math.cos math.sin | -: real);
            ! print ,. (~ (&& (|| (~ flags) 0b) 1b));
        };"#,
    );
    let attempt = |optimize, resources| {
        run_with_options(
            &program,
            RuntimeOptions {
                resources,
                optimize,
            },
        )
        .map(|output| output.stdout)
        .map_err(|err| {
            let diag = err.structured().unwrap();
            (diag.primary, diag.category, err.message)
        })
    };
    let default = ResourcePolicy::default();
    let expected = attempt(false, default).expect("ordinary checked evaluation succeeds");
    assert_eq!(attempt(true, default).unwrap(), expected);
    assert!(expected.ends_with("[0b 1b 0b 1b 0b 1b]\n"));
    for max_work in 1..180 {
        let limits = ResourcePolicy {
            max_work,
            ..default
        };
        assert_eq!(attempt(true, limits), attempt(false, limits), "{limits:?}");
    }
    for max_depth in 1..20 {
        let limits = ResourcePolicy {
            max_depth,
            ..default
        };
        assert_eq!(attempt(true, limits), attempt(false, limits), "{limits:?}");
    }
}

#[test]
fn tight_tracked_memory_replays_without_repeating_effectful_leaf() {
    let program = checked(
        r#"leaf := !^{ ! print "once"; form [2n] [-0.0r 2r] };
        !^{ ! print (-: | -: (^? ! leaf)); };"#,
    );
    for max_memory in [32, ResourcePolicy::default().max_memory] {
        let options = RuntimeOptions {
            resources: ResourcePolicy {
                max_memory,
                ..ResourcePolicy::default()
            },
            optimize: true,
        };
        let output = run_with_options(&program, options).unwrap();
        assert_eq!(output.stdout, "once\narray [2] [-0 -2]\n");
    }
}

#[test]
fn real_scalar_proof_is_total_without_losing_sign_or_rounding() {
    use super::ir::Kernel;
    let source = r#"
        a := form [4n] [-0.0r 0.0r 1.7976931348623157e308r -5e-324r];
        x := + (- (* (/ (:r a) -2r) -0.5r) 0r) -0.0r;
        !^{ ! print x; };"#;
    let program = checked(source);
    let root = find(&program, CoreOp::Add);
    let plan = plan(&program, root, 128).unwrap();
    assert_eq!(plan.stages().len(), 5);
    assert_eq!(
        (plan.real(-0.0).to_bits(), plan.real(0.0).to_bits()),
        ((-0.0_f64).to_bits(), 0)
    );
    assert!(plan.real(f64::MAX).is_finite());
    assert!(plan.real(-f64::MAX).is_finite());
    let mut wrong = lowering::build(&program, root, 128).unwrap();
    let Kernel::RealScalar { literal, .. } = &mut wrong.stages[0].kernel else {
        panic!("real literal proof")
    };
    literal.bits = 0;
    assert!(CheckedPlan::validate(&program, wrong, 128).is_none());

    let attempt = |optimize, resources| {
        run_with_options(
            &program,
            RuntimeOptions {
                resources,
                optimize,
            },
        )
        .map(|out| out.stdout)
        .map_err(|error| (error.start, error.message))
    };
    let default = ResourcePolicy::default();
    assert_eq!(attempt(true, default), attempt(false, default));
    for max_work in 1..75 {
        let limits = ResourcePolicy {
            max_work,
            ..default
        };
        assert_eq!(attempt(true, limits), attempt(false, limits), "{limits:?}");
    }

    for (expr, op) in [
        ("+ (-: a) 1r", CoreOp::Add),
        ("* (-: a) 1.0000000000000002r", CoreOp::Mul),
        ("/ (-: a) -0.5r", CoreOp::Div),
        ("/ (-: a) -0.0r", CoreOp::Div),
        ("- (-: a) scalar", CoreOp::Sub),
        ("+ (-: a) (form [1n] [0r])", CoreOp::Add),
    ] {
        let program = checked(&format!("scalar := 0r; a := form [1n] [1r]; x := {expr};"));
        assert!(
            lowering::build(&program, find(&program, op), 128).is_none(),
            "{expr}"
        );
    }
}
