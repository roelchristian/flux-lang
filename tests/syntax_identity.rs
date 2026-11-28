use flux_lang::{hir, infer, parser, resolve, semantic};

const SOURCE: &str = r#"
id(x:'a):'a := x;
copy:z->z := &{ @* + . . };
!{
    ^{
        ^? ! print (id 1z, @ copy 1z);
        ()
    }
};
"#;

#[test]
fn syntax_identity_is_occurrence_based_and_survives_moves_and_reparse() {
    let source = parser::parse(SOURCE).unwrap();
    let res = resolve::resolve(&source).unwrap();
    let inferred = infer::infer(&source, &res).unwrap();
    let clone = source.clone();
    let reparsed = parser::parse(SOURCE).unwrap();
    let ids: Vec<_> = source.identified_expressions().collect();
    assert_eq!(ids, clone.identified_expressions().collect::<Vec<_>>());
    assert_eq!(ids, reparsed.identified_expressions().collect::<Vec<_>>());
    assert!(ids.iter().any(|&(id, _)| res.span_at(id).is_none())); // tacit expansion
    let equal_literals: Vec<_> = ids
        .iter()
        .filter(|(_, e)| matches!(e, parser::Expr::Num(n) if n == "1z"))
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(equal_literals.len(), 2);
    assert_ne!(equal_literals[0], equal_literals[1]);
    for (id, expr) in ids {
        assert_eq!(res.syntax_id(expr), Some(id));
        assert_eq!(res.reference(expr), res.reference_at(id));
        assert_eq!(res.span(expr), res.span_at(id));
        assert_eq!(inferred.type_of(expr), inferred.type_at(id));
        assert_eq!(inferred.callee_type(expr), inferred.callee_type_at(id));
        assert_eq!(inferred.boundary_of(expr), inferred.boundary_at(id));
    }
    let h = semantic::prepare(&clone, semantic::Mode::Script, semantic::Policy::TypedOnly).unwrap();
    let mut seen = std::collections::HashSet::new();
    for node in &h.hir().unwrap().arena {
        assert!(seen.insert(node.source_id));
        assert_eq!(inferred.type_at(node.source_id), Some(&node.ty));
    }
}

#[test]
fn checked_metadata_rebinds_to_owned_clones_but_rejects_edits() {
    let program = parser::parse(SOURCE).unwrap();
    let res = resolve::resolve(&program).unwrap();
    let clone = program.clone();
    let inferred = infer::infer(&clone, &res).unwrap();
    let reparsed = parser::parse(SOURCE).unwrap();
    hir::lower(&reparsed, &inferred, &res).unwrap();

    let mut changed = reparsed;
    let parser::Stmt::Def { name, .. } = &mut changed.defs[0] else {
        panic!("fixture must begin with a named declaration");
    };
    *name = "not_the_original".into();
    let err = infer::infer(&changed, &res).unwrap_err();
    assert!(err.message.contains("different syntax tree"));
    let err = hir::lower(&changed, &inferred, &res).unwrap_err();
    assert!(err.message.contains("different syntax tree"));
}

#[test]
fn rebind_refreshes_exact_source_spans() {
    let original = parser::parse("x := +  1z 2z;").unwrap();
    let respaced = parser::parse("x := + 1z  2z;").unwrap();
    let res = resolve::resolve(&original).unwrap();
    let inf = infer::infer(&respaced, &res).unwrap();
    let lowered = hir::lower(&respaced, &inf, &res).unwrap();
    let expected = resolve::resolve(&respaced).unwrap();
    for node in &lowered.arena {
        let expected = expected.span_at(node.source_id).unwrap();
        assert_eq!(node.span.start..node.span.end, expected);
    }
}
