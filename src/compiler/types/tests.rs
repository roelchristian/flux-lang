use super::{Class, Constraint, InferCtx, NominalId, Prim, Row, Scheme, Type};
use std::collections::BTreeSet;

fn t(p: Prim) -> Type {
    Type::Prim(p)
}
fn row(fields: &[(&str, Type)], tail: Option<u32>) -> Type {
    Type::Record(Row {
        fields: fields
            .iter()
            .map(|(n, t)| (n.to_string(), t.clone()))
            .collect(),
        tail,
    })
}

#[test]
fn unifies_through_constructors_and_rejects_cycles() {
    let mut ctx = InferCtx::new();
    let a = ctx.fresh();
    let b = ctx.fresh();
    let x = Type::Fun(
        Box::new(a.clone()),
        Box::new(Type::Optional(Box::new(b.clone()))),
    );
    let y = Type::Fun(
        Box::new(t(Prim::S)),
        Box::new(Type::Optional(Box::new(t(Prim::Q)))),
    );
    ctx.unify(&x, &y).unwrap();
    assert_eq!(ctx.resolve(&x), y);
    let c = ctx.fresh();
    assert!(ctx
        .unify(&c, &Type::Seq(Box::new(c.clone())))
        .unwrap_err()
        .contains("occurs check"));
    assert_eq!(ctx.resolve(&c), c);
}

#[test]
fn quantified_instantiations_are_independent_and_constraints_rename() {
    let mut ctx = InferCtx::new();
    let a = ctx.fresh();
    let env_ty = ctx.fresh();
    let fun = Type::Fun(Box::new(a.clone()), Box::new(env_ty.clone()));
    ctx.require(Class::Num, &a).unwrap();
    let s = ctx.generalize(&fun, &[Scheme::mono(env_ty.clone())], &ctx.constraints());
    assert_eq!(s.vars.len(), 1);
    let (l, lc) = ctx.instantiate(&s);
    let (r, rc) = ctx.instantiate(&s);
    assert_ne!(l, r);
    assert_ne!(lc, rc);
    assert_eq!(lc[0].class, Class::Num);
    ctx.require(lc[0].class, &lc[0].ty).unwrap();
    assert!(ctx.unify(&lc[0].ty, &t(Prim::C)).is_err());
    assert_eq!(ctx.resolve(&lc[0].ty), lc[0].ty);
    ctx.unify(&lc[0].ty, &t(Prim::D)).unwrap();
    assert_eq!(ctx.resolve(&rc[0].ty), rc[0].ty);
}

#[test]
fn open_rows_merge_then_close_without_losing_fields() {
    let mut ctx = InferCtx::new();
    let a = ctx.fresh_row();
    let b = ctx.fresh_row();
    let field = ctx.fresh();
    let x = row(&[("y", t(Prim::R)), ("x", field.clone())], Some(a));
    let y = row(&[("x", t(Prim::Z)), ("z", t(Prim::B))], Some(b));
    ctx.unify(&x, &y).unwrap();
    let z = row(
        &[("z", t(Prim::B)), ("x", t(Prim::Z)), ("y", t(Prim::R))],
        None,
    );
    ctx.unify(&x, &z).unwrap();
    let Type::Record(resolved) = ctx.resolve(&x) else {
        panic!()
    };
    assert_eq!(resolved.tail, None);
    assert_eq!(
        resolved
            .fields
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>(),
        vec!["y", "x", "z"]
    );
    assert_eq!(ctx.resolve(&field), t(Prim::Z));
    assert!(ctx.unify(&x, &row(&[("x", t(Prim::Z))], None)).is_err());
}

#[test]
fn rows_reject_duplicate_labels_cycles_and_conflicting_common_types() {
    for swap in [false, true] {
        let mut ctx = InferCtx::new();
        let a = ctx.fresh_row();
        let b = ctx.fresh_row();
        let x = row(&[("x", t(Prim::Z))], Some(a));
        let y = row(&[("x", t(Prim::B))], Some(b));
        assert!(ctx.unify(&x, &y).unwrap_err().contains("field x"));
        assert!(ctx
            .unify(&x, &row(&[("x", t(Prim::Z)), ("y", t(Prim::Z))], Some(a)))
            .is_err());
        let alias = row(&[], Some(a));
        let dup = row(&[("x", t(Prim::Z))], None);
        let err = if swap {
            ctx.unify(&dup, &alias)
        } else {
            ctx.unify(&alias, &dup)
        };
        // The failed unifications cannot have registered a lacks constraint.
        assert!(err.is_ok());
        assert!(ctx
            .unify(&x, &x)
            .unwrap_err()
            .contains("duplicate row field x"));
    }
}

#[test]
fn row_quantification_freshens_tails() {
    let mut ctx = InferCtx::new();
    let a = ctx.fresh_row();
    let ty = row(&[("x", t(Prim::Z))], Some(a));
    let scheme = ctx.generalize(&ty, &[], &[]);
    let (l, _) = ctx.instantiate(&scheme);
    let (r, _) = ctx.instantiate(&scheme);
    assert_ne!(l, r);
    ctx.unify(&l, &row(&[("x", t(Prim::Z)), ("y", t(Prim::Q))], None))
        .unwrap();
    assert_eq!(ctx.resolve(&r), r);
}

#[test]
fn composite_requirements_normalize_before_and_after_binding() {
    let mut cx = InferCtx::new();
    let a = cx.fresh();
    let b = cx.fresh();
    let c = cx.fresh();
    cx.require(
        Class::Eq,
        &Type::Seq(Box::new(Type::Optional(Box::new(a.clone())))),
    )
    .unwrap();
    cx.require(Class::Hash, &Type::Optional(Box::new(b.clone())))
        .unwrap();
    assert_eq!(
        cx.constraints(),
        vec![
            Constraint {
                class: Class::Eq,
                ty: a.clone()
            },
            Constraint {
                class: Class::Hash,
                ty: b.clone()
            },
        ]
    );
    cx.unify(&a, &Type::Tuple(vec![c.clone(), t(Prim::B)]))
        .unwrap();
    assert_eq!(
        cx.constraints()[0],
        Constraint {
            class: Class::Eq,
            ty: c.clone()
        }
    );
    let fun = Type::Fun(Box::new(t(Prim::N)), Box::new(t(Prim::N)));
    assert!(cx
        .unify(&c, &fun)
        .unwrap_err()
        .contains("does not satisfy eq"));
    assert!(cx
        .unify(&b, &fun)
        .unwrap_err()
        .contains("does not satisfy hash"));
    cx.unify(&c, &t(Prim::Z)).unwrap();
    cx.unify(&b, &t(Prim::S)).unwrap();
    assert!(cx.constraints().is_empty());
}

#[test]
fn open_record_equality_normalizes_new_fields_after_row_binding() {
    let mut cx = InferCtx::new();
    let tail = cx.fresh_row();
    let a = cx.fresh();
    let b = cx.fresh();
    let open = row(&[("a", a.clone())], Some(tail));
    cx.require(Class::Eq, &open).unwrap();
    assert_eq!(
        cx.constraints(),
        vec![
            Constraint {
                class: Class::Eq,
                ty: a.clone()
            },
            Constraint {
                class: Class::Eq,
                ty: row(&[], Some(tail))
            },
        ]
    );
    cx.unify(&open, &row(&[("a", t(Prim::C)), ("b", b.clone())], None))
        .unwrap();
    assert_eq!(
        cx.constraints(),
        vec![Constraint {
            class: Class::Eq,
            ty: b.clone()
        }]
    );
    let action = Type::Action(Box::new(t(Prim::Unit)));
    assert!(cx
        .unify(&b, &action)
        .unwrap_err()
        .contains("does not satisfy eq"));
}

#[test]
fn rigid_variables_reject_specialization_and_cannot_escape_as_outer_bindings() {
    let mut cx = InferCtx::new();
    let outer = cx.fresh();
    let fresh = cx.fresh();
    let rigid = cx.fresh_rigid();
    let other_rigid = cx.fresh_rigid();
    let Type::Var(rid) = rigid else { panic!() };
    assert!(cx.unify(&rigid, &other_rigid).is_err());
    assert!(cx
        .unify(&rigid, &t(Prim::Z))
        .unwrap_err()
        .contains(&format!("annotated type variable ~t{rid}")));
    assert!(cx
        .unify(&Type::Optional(Box::new(t(Prim::Z))), &rigid)
        .is_err());
    cx.unify(&rigid, &fresh).unwrap();
    assert_eq!(cx.resolve(&fresh), rigid);
    assert!(!cx.rigid_escapes(&[rid], &[Scheme::mono(outer.clone())]));
    cx.unify(&outer, &rigid).unwrap();
    assert!(cx.rigid_escapes(&[rid], &[Scheme::mono(outer)]));
    let scheme = cx.generalize(&rigid, &[], &[]);
    assert_eq!(scheme.vars, vec![rid]);
    assert_ne!(cx.instantiate(&scheme).0, rigid);
    cx.unmark_rigid(rid);
    cx.unify(&rigid, &t(Prim::Z)).unwrap();
}

#[test]
fn rigid_row_tails_can_be_used_but_not_closed_or_strengthened() {
    let mut cx = InferCtx::new();
    let skolem = cx.fresh_row();
    let observed = row(&[("k", t(Prim::S)), ("v", t(Prim::Z))], Some(skolem));
    cx.register(&observed).unwrap();
    cx.mark_rigid(skolem);
    let flex = cx.fresh_row();
    let wanted = row(&[("k", t(Prim::S))], Some(flex));
    cx.unify(&observed, &wanted).unwrap();
    assert_eq!(cx.resolve(&wanted), observed);
    let closed = row(&[("k", t(Prim::S)), ("v", t(Prim::Z))], None);
    assert!(cx
        .unify(&observed, &closed)
        .unwrap_err()
        .contains(&format!("annotated row variable ~r{skolem}")));
    let extra = row(&[("new", t(Prim::Z))], Some(cx.fresh_row()));
    assert!(cx.unify(&observed, &extra).is_err());
    let strengthened = row(&[("extra", t(Prim::Z))], Some(skolem));
    assert!(cx
        .register(&strengthened)
        .unwrap_err()
        .contains("not known to lack field extra"));
}

#[test]
fn rows_and_value_variables_cannot_reuse_an_identifier() {
    for row_first in [false, true] {
        let mut cx = InferCtx::new();
        let ordinary = Type::Var(32);
        let record = row(&[], Some(32));
        let (first, other) = if row_first {
            (&record, &ordinary)
        } else {
            (&ordinary, &record)
        };
        cx.register(first).unwrap();
        assert!(cx
            .register(other)
            .unwrap_err()
            .contains("both type and row kind"));
    }
}

#[test]
fn a_scheme_replays_hidden_lacks_in_a_different_context() {
    let mut declaration = InferCtx::new();
    let id = declaration.fresh_row();
    let hidden_requirement = row(&[("reserved", t(Prim::B))], Some(id));
    declaration.register(&hidden_requirement).unwrap();
    let visible = row(&[("value", t(Prim::Q))], Some(id));
    let scheme = declaration.generalize(&visible, &[], &[]);
    assert_eq!(
        scheme.row_lacks[&id],
        BTreeSet::from(["reserved".into(), "value".into()])
    );

    let mut uses = InferCtx::new();
    let (bad, _) = uses.instantiate(&scheme);
    let forbidden = row(&[("value", t(Prim::Q)), ("reserved", t(Prim::B))], None);
    assert!(uses
        .unify(&bad, &forbidden)
        .unwrap_err()
        .contains("duplicate row field reserved"));
    let (good, _) = uses.instantiate(&scheme);
    let allowed = row(&[("value", t(Prim::Q)), ("label", t(Prim::S))], None);
    uses.unify(&good, &allowed).unwrap();
}

#[test]
fn nominal_identity_is_not_shared_by_shadowed_spelling() {
    let mut cx = InferCtx::new();
    let nominal = |id| {
        Type::Nominal(NominalId {
            id,
            name: "row".into(),
        })
    };
    assert_eq!(nominal(1).to_string(), nominal(2).to_string());
    assert!(cx.unify(&nominal(1), &nominal(2)).is_err());
    cx.unify(&nominal(1), &nominal(1)).unwrap();
}

#[test]
fn nominal_applications_preserve_identity_and_instantiate_arguments() {
    let mut cx = InferCtx::new();
    let nominal = |id, arg| {
        Type::Applied(
            NominalId {
                id,
                name: "tree".into(),
            },
            vec![arg],
        )
    };
    let arg = cx.fresh();
    let body = nominal(1, arg.clone());
    let scheme = cx.generalize(&body, &[], &[]);
    assert_eq!(scheme.vars.len(), 1);
    assert!(cx.unify(&arg, &body).is_err());
    let (first, _) = cx.instantiate(&scheme);
    let (second, _) = cx.instantiate(&scheme);
    cx.unify(&first, &nominal(1, t(Prim::N))).unwrap();
    cx.unify(&second, &nominal(1, t(Prim::S))).unwrap();
    assert_eq!(cx.resolve(&first).to_string(), "tree[n]");
    assert_eq!(cx.resolve(&second).to_string(), "tree[s]");
    assert!(cx.unify(&first, &nominal(2, t(Prim::N))).is_err());
    assert!(cx.require(Class::Eq, &first).is_err());
}

#[test]
fn registering_public_unallocated_ids_reserves_them_and_rolls_back_on_failure() {
    let mut cx = InferCtx::new();
    cx.unify(&Type::Var(101), &t(Prim::Z)).unwrap();
    assert_eq!(cx.fresh(), Type::Var(102));
    assert!(cx.unify(&Type::Var(u32::MAX), &t(Prim::Z)).is_err());
    assert_eq!(cx.fresh(), Type::Var(103));
    let tail = 141;
    let open = row(&[("k", Type::Var(140))], Some(tail));
    let alias = Type::Var(131);
    cx.unify(&alias, &open).unwrap();
    let tailless = row(&[("k", t(Prim::Z)), ("v", t(Prim::B))], None);
    cx.unify(&alias, &tailless).unwrap();
    assert_eq!(cx.resolve(&alias), tailless);
    assert_eq!(cx.fresh(), Type::Var(142));
}

#[test]
fn formatting_is_ordered_and_parenthesizes_function_operands() {
    let fields = row(&[("z", t(Prim::R)), ("a", t(Prim::D))], Some(7));
    assert_eq!(fields.to_string(), ".{z:r; a:d; | ~r7}");
    let fn_ty = Type::Fun(Box::new(t(Prim::N)), Box::new(t(Prim::Q)));
    assert_eq!(Type::Action(Box::new(fn_ty)).to_string(), "!(n->q)");
}
#[test]
fn result_types_substitute_and_preserve_precedence() {
    let mut cx = InferCtx::new();
    let ok = cx.fresh();
    let err = cx.fresh();
    let ty = Type::Result(Box::new(ok.clone()), Box::new(err.clone()));
    assert_eq!(cx.free_vars(&ty).len(), 2);
    cx.unify(&ok, &t(Prim::Q)).unwrap();
    cx.unify(&err, &t(Prim::S)).unwrap();
    assert_eq!(cx.resolve(&ty).to_string(), "q^s");
    assert_eq!(
        Type::Action(Box::new(cx.resolve(&ty))).to_string(),
        "!(q^s)"
    );
    let inverse = Type::Result(
        Box::new(Type::Action(Box::new(t(Prim::Q)))),
        Box::new(t(Prim::S)),
    );
    assert_eq!(inverse.to_string(), "!q^s");
    let chained = Type::Fun(Box::new(cx.resolve(&ty)), Box::new(inverse));
    assert_eq!(chained.to_string(), "q^s->!q^s");
}
