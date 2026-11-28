use flux_lang::{
    diagnostic::Category,
    hir::{self, CoreOp, ExprId, ExprKind, LiftMode, Target},
    hir_eval, parser,
    semantic::{self, Mode, Policy},
};

fn checked(source: &str) -> hir::HirProgram {
    let program = parser::parse(source).unwrap();
    semantic::prepare(&program, Mode::Script, Policy::TypedOnly)
        .unwrap()
        .hir()
        .unwrap()
        .clone()
}

#[test]
fn malformed_public_hir_fails_before_any_output() {
    let mut p = checked("!{ ^= ! print \"not emitted\" { _ : (); }; };");
    let entry = p.entry.unwrap();
    p.arena[entry.0].kind = ExprKind::Tuple(vec![entry]);
    let mut sink = Vec::new();
    let err = hir_eval::run_with_output(&p, &mut sink).unwrap_err();
    assert_eq!(err.structured().unwrap().category, Category::InvalidHir);
    assert!(err.message.contains("expression reference"));
    assert!(sink.is_empty());
    assert!(hir_eval::view_entry(&p).is_err());
}

#[test]
fn invalid_lift_and_wrong_builtin_arity_are_distinct() {
    let p = checked("x := + [1z 2z] 3z;");
    let i = p
        .arena
        .iter()
        .position(|n| matches!(&n.kind, ExprKind::Call(Target::Builtin(CoreOp::Add), _)))
        .unwrap();
    let mut no_witness = p.clone();
    no_witness.arena[i].evidence = None;
    assert!(hir::validate(&no_witness)
        .unwrap_err()
        .message
        .contains("arithmetic witness"));
    let mut bad_plan = p.clone();
    bad_plan.arena[i].lift_mode = Some(LiftMode::Scalar);
    assert!(hir::validate(&bad_plan)
        .unwrap_err()
        .message
        .contains("lifting evidence"));
    let mut bad_arity = p;
    let ExprKind::Call(_, args) = &mut bad_arity.arena[i].kind else {
        unreachable!()
    };
    args.pop();
    assert!(hir::validate(&bad_arity)
        .unwrap_err()
        .message
        .contains("expects 2"));
}

#[test]
fn missing_entry_or_call_child_is_not_an_index_panic() {
    let mut p = checked("!{ () };");
    p.entry = Some(ExprId(usize::MAX));
    let err = hir_eval::run(&p).unwrap_err();
    assert_eq!(err.structured().unwrap().category, Category::InvalidHir);
}

#[test]
fn generic_valid_hir_preserves_each_checked_application() {
    let src = r#"
id(x:'a):'a := x;
both := (id 8n,id "well");
square := &{ @* * . . };
!{ ^= ! print (both,@ square 3q) { _ : (); }; };
"#;
    let p = checked(src);
    hir::validate(&p).expect("generic HIR with polymorphic and tacit applications");
    assert_eq!(hir_eval::run(&p).unwrap().stdout, "((8, well), 9)\n");
}

#[test]
fn rejects_cross_closure_load_and_missing_concrete_schema() {
    let p = checked("aa(x:z):z := x; bb(y:z):z := y; pair := (aa 1z,bb 2z);");
    let loads = p
        .arena
        .iter()
        .enumerate()
        .filter_map(|(i, node)| {
            if let ExprKind::Load(id) = node.kind {
                Some((i, id))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    assert_eq!(loads.len(), 2);
    let mut malformed = p;
    malformed.arena[loads[1].0].kind = ExprKind::Load(loads[0].1);
    assert!(hir::validate(&malformed)
        .unwrap_err()
        .message
        .contains("lexical owner"));

    let mut missing = checked("row :: { key:z; }; t:#row := #[];");
    let table = missing
        .arena
        .iter_mut()
        .find(|n| n.table_schema.is_some())
        .unwrap();
    table.table_schema = None;
    assert!(hir::validate(&missing)
        .unwrap_err()
        .message
        .contains("table layout"));
}

#[test]
fn escapes_cannot_skip_the_nearest_boundary_or_cross_an_action() {
    for body in [
        r#"inner:()^ioerr := ^{ ^: missing "inner" }; ^? inner;"#,
        r#"inner:!(()^ioerr) := !{ ^{ ^: missing "inner" } }; ^? ! inner;"#,
    ] {
        let mut p = checked(&format!("!{{ ^{{ {body} () }} }};"));
        let outer = p
            .arena
            .iter()
            .filter_map(|n| match n.kind {
                ExprKind::Fallible { boundary, .. } => Some(boundary),
                _ => None,
            })
            .last()
            .unwrap();
        let raised = p
            .arena
            .iter_mut()
            .find(|n| matches!(n.kind, ExprKind::Raise { .. }))
            .unwrap();
        let ExprKind::Raise { boundary, .. } = &mut raised.kind else {
            unreachable!()
        };
        assert_ne!(*boundary, outer);
        *boundary = outer;
        assert!(hir::validate(&p)
            .unwrap_err()
            .message
            .contains("nearest lexical"));
    }
}
