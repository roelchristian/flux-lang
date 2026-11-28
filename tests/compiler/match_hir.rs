use super::hir_validation::checked;
use flux_lang::{
    diagnostic::Category,
    hir::{self, ExprKind, ValuePatternKind},
    hir_eval,
};

#[test]
fn match_rejects_mutated_cross_arm_binder_before_execution() {
    let mut p = checked("!{ ^{ n := ?= (?1 0z) { ?0 : 0z; ?1 x : x; }; ^? ! print n; () } };");
    let (first, binding) = p
        .arena
        .iter()
        .find_map(|node| {
            let ExprKind::Match { arms, .. } = &node.kind else {
                return None;
            };
            let ValuePatternKind::Optional(Some(payload)) = &arms[1].pattern.kind else {
                unreachable!()
            };
            let ValuePatternKind::Bind(binding) = payload.kind else {
                unreachable!()
            };
            Some((arms[0].body, binding))
        })
        .unwrap();
    p.arena[first.0].kind = ExprKind::Load(binding);
    let mut output = Vec::new();
    let error = hir_eval::run_with_output(&p, &mut output).unwrap_err();
    assert_eq!(error.structured().unwrap().category, Category::InvalidHir);
    assert!(error.message.contains("lexical owner"), "{}", error.message);
    assert!(output.is_empty());
}

#[test]
fn constructor_pattern_witness_has_checked_arity_and_stable_identity() {
    let mut p = checked(
        "boxx[~a] :: { | item ~a; };          first(v:boxx[~a]):~a := ?= v { item x : x; };          read:boxx[z]->z := &first;          !{ ^{ ^? ! print (@ read (item 41z)); () } };",
    );
    assert_eq!(hir_eval::run(&p).unwrap().stdout, "41\n");
    let arm = p
        .arena
        .iter_mut()
        .find_map(|node| {
            let ExprKind::Match { arms, .. } = &mut node.kind else {
                return None;
            };
            arms.first_mut()
        })
        .unwrap();
    let ValuePatternKind::Constructor(_, fields) = &mut arm.pattern.kind else {
        panic!("expected constructor pattern");
    };
    fields.clear();
    let error = hir::validate(&p).unwrap_err();
    assert!(
        error.message.contains("match constructor arity"),
        "{}",
        error.message
    );
}

#[test]
fn callable_instances_project_numeric_types_through_generic_sums() {
    let p = checked(
        "boxx[~a] :: { | item ~a; };          doubled(v:boxx[~a]):~a | num ~a := ?= v { item x : + x x; };          !{ ^{ ^? ! print (@ &doubled (item 1.20d), @ &doubled (item 7z)); () } };",
    );
    assert_eq!(hir_eval::run(&p).unwrap().stdout, "(2.40, 14)\n");
}
