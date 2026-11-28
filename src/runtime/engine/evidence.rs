//! Instantiate checked type and row evidence for a lexical callable application.
use super::Eval;
use crate::{
    hir::{CoreOp, LiftMode},
    hir_eval::{NumericResult, V},
    types::Type,
};
use std::{collections::HashMap, rc::Rc};

pub(super) type RowInstances = HashMap<u32, RowInstance>;
pub(super) struct RowInstance {
    /// Only tail-owned fields may supply field types to the output occurrence.
    residual: crate::types::Row,
    /// Preserve the checked input's interleaving of required and extra fields.
    order: Vec<String>,
    fixed: Vec<String>,
}

pub(super) fn contains_open_row(ty: &Type) -> bool {
    match ty {
        Type::Record(row) => {
            row.tail.is_some() || row.fields.iter().any(|(_, ty)| contains_open_row(ty))
        }
        Type::Seq(t) | Type::Table(t) | Type::Optional(t) | Type::Action(t) => contains_open_row(t),
        Type::Dict(a, b) | Type::Fun(a, b) | Type::Result(a, b) => {
            contains_open_row(a) || contains_open_row(b)
        }
        Type::Tuple(ts) | Type::Applied(_, ts) => ts.iter().any(contains_open_row),
        _ => false,
    }
}
pub(super) fn dynamic_schema_operation(op: CoreOp) -> bool {
    matches!(
        op,
        CoreOp::TableFrom
            | CoreOp::TableRows
            | CoreOp::Aggregate
            | CoreOp::Extend
            | CoreOp::Cartesian
            | CoreOp::CsvRead
    )
}
pub(super) fn fit_instance(
    pattern: &Type,
    actual: &Type,
    vars: &mut HashMap<u32, Type>,
    rows: &mut RowInstances,
) {
    match (pattern, actual) {
        (Type::Var(i), t) if !matches!(t, Type::Var(_)) => {
            vars.entry(*i).or_insert_with(|| t.clone());
        }
        (Type::Seq(a), Type::Seq(b))
        | (Type::Table(a), Type::Table(b))
        | (Type::Optional(a), Type::Optional(b))
        | (Type::Action(a), Type::Action(b)) => fit_instance(a, b, vars, rows),
        (Type::Dict(a, b), Type::Dict(c, d))
        | (Type::Fun(a, b), Type::Fun(c, d))
        | (Type::Result(a, b), Type::Result(c, d)) => {
            fit_instance(a, c, vars, rows);
            fit_instance(b, d, vars, rows);
        }
        (Type::Applied(a, args), Type::Applied(b, actual))
            if a == b && args.len() == actual.len() =>
        {
            for (arg, actual) in args.iter().zip(actual) {
                fit_instance(arg, actual, vars, rows);
            }
        }
        (Type::Tuple(a), Type::Tuple(b)) => {
            for (a, b) in a.iter().zip(b) {
                fit_instance(a, b, vars, rows);
            }
        }
        (Type::Record(a), Type::Record(b)) => {
            if let Some(tail) = a.tail {
                // An identical unresolved tail is not additional evidence.
                if b.tail != Some(tail)
                    || b.fields
                        .iter()
                        .any(|(name, _)| !a.fields.iter().any(|(fixed, _)| fixed == name))
                {
                    let capture = || RowInstance {
                        residual: crate::types::Row {
                            fields: b
                                .fields
                                .iter()
                                .filter(|(name, _)| {
                                    !a.fields.iter().any(|(fixed, _)| fixed == name)
                                })
                                .cloned()
                                .collect(),
                            tail: b.tail,
                        },
                        order: b.fields.iter().map(|(name, _)| name.clone()).collect(),
                        fixed: a.fields.iter().map(|(name, _)| name.clone()).collect(),
                    };
                    match rows.entry(tail) {
                        std::collections::hash_map::Entry::Vacant(slot) => {
                            slot.insert(capture());
                        }
                        std::collections::hash_map::Entry::Occupied(mut slot)
                            if slot.get().residual.tail.is_some() && b.tail.is_none() =>
                        {
                            slot.insert(capture());
                        }
                        _ => {}
                    }
                }
            }
            for (n, a) in &a.fields {
                if let Some((_, b)) = b.fields.iter().find(|(m, _)| n == m) {
                    fit_instance(a, b, vars, rows);
                }
            }
        }
        _ => {}
    }
}
pub(in crate::hir_eval) fn fun_arg(mut sig: &Type, i: usize) -> Option<&Type> {
    for _ in 0..i {
        if let Type::Fun(_, next) = sig {
            sig = next
        } else {
            return None;
        }
    }
    if let Type::Fun(arg, _) = sig {
        Some(arg)
    } else {
        None
    }
}
pub(super) fn callable_lift(op: CoreOp, operands: &[Type]) -> NumericResult<Option<LiftMode>> {
    let Some(arity) = op.lift_arity() else {
        return Ok(None);
    };
    if operands.len() != arity {
        return Err("invalid typed lifting argument count");
    }
    if operands.iter().any(|ty| matches!(ty, Type::Var(_))) {
        return Err("numeric lift shape is unresolved at indirect application");
    }
    LiftMode::for_operands(&operands[0], operands.get(1)).map(Some)
}
pub(in crate::hir_eval) fn inner_type(t: &Type) -> &Type {
    if let Type::Seq(x) = t {
        x
    } else {
        t.array_element().unwrap_or(t)
    }
}
pub(super) fn result_type(mut t: &Type, n: usize) -> &Type {
    for _ in 0..n {
        if let Type::Fun(_, r) = t {
            t = r
        } else {
            break;
        }
    }
    t
}

impl Eval<'_> {
    pub(super) fn type_at(&self, ty: &Type, env: usize) -> Type {
        fn subst(e: &Eval<'_>, ty: &Type, scope: Option<usize>, fuel: usize) -> Type {
            if scope.is_none() || fuel == 0 {
                return ty.clone();
            }
            let recur = |t: &Type| subst(e, t, scope, fuel - 1);
            match ty {
                Type::Var(id) => {
                    let mut cursor = scope;
                    while let Some(i) = cursor {
                        let frame = &e.frames[i];
                        if let Some(t) = frame.substs.get(id) {
                            return subst(
                                e,
                                t,
                                frame.parent.and_then(|p| e.frames[p].type_scope),
                                fuel - 1,
                            );
                        }
                        cursor = frame.parent.and_then(|p| e.frames[p].type_scope);
                    }
                    ty.clone()
                }
                Type::Seq(t) => Type::Seq(Box::new(recur(t))),
                Type::Table(t) => Type::Table(Box::new(recur(t))),
                Type::Optional(t) => Type::Optional(Box::new(recur(t))),
                Type::Action(t) => Type::Action(Box::new(recur(t))),
                Type::Tuple(ts) => Type::Tuple(ts.iter().map(recur).collect()),
                Type::Fun(a, b) => Type::Fun(Box::new(recur(a)), Box::new(recur(b))),
                Type::Result(a, b) => Type::Result(Box::new(recur(a)), Box::new(recur(b))),
                Type::Dict(a, b) => Type::Dict(Box::new(recur(a)), Box::new(recur(b))),
                Type::Applied(owner, args) => {
                    Type::Applied(owner.clone(), args.iter().map(recur).collect())
                }
                Type::Record(r) => {
                    let mut r = r.clone();
                    for (_, t) in &mut r.fields {
                        *t = recur(t);
                    }
                    let mut cursor = scope;
                    let mut steps = fuel;
                    while let (Some(tail), Some(i)) = (r.tail, cursor) {
                        if steps == 0 {
                            break;
                        }
                        steps -= 1;
                        let frame = &e.frames[i];
                        cursor = frame.parent.and_then(|p| e.frames[p].type_scope);
                        let Some(witness) =
                            frame.row_substs.as_ref().and_then(|rows| rows.get(&tail))
                        else {
                            continue;
                        };
                        let preserve_interleaving = witness
                            .fixed
                            .iter()
                            .all(|fixed| r.fields.iter().any(|(name, _)| fixed == name));
                        for (name, t) in &witness.residual.fields {
                            if !r.fields.iter().any(|(existing, _)| existing == name) {
                                r.fields.push((name.clone(), subst(e, t, cursor, steps)));
                            }
                        }
                        if preserve_interleaving {
                            // Existing columns retain their checked input order; fields
                            // introduced by this expression follow in declaration order.
                            r.fields.sort_by_key(|(name, _)| {
                                witness
                                    .order
                                    .iter()
                                    .position(|original| original == name)
                                    .unwrap_or(usize::MAX)
                            });
                        }
                        r.tail = witness.residual.tail;
                    }
                    Type::Record(r)
                }
                _ => ty.clone(),
            }
        }
        subst(self, ty, self.frames[env].type_scope, 64)
    }
    pub(super) fn specialize_fun(&self, v: V, ty: &Type, env: usize) -> V {
        if let V::Fun(f) = v {
            if matches!(ty, Type::Fun(..) | Type::Var(_)) {
                let ty = self.type_at(ty, env);
                if matches!(ty, Type::Fun(..)) && f.instance.as_ref() != Some(&ty) {
                    let mut f = Rc::unwrap_or_clone(f);
                    f.instance = Some(ty);
                    return V::Fun(Rc::new(f));
                }
            }
            V::Fun(f)
        } else {
            v
        }
    }
}
