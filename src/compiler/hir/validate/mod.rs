//! Structural admission checks for stand-alone typed HIR.
mod lexical;
mod operations;
use super::{
    ConstructorDef, CoreOp, Definition, ExprId, ExprKind, HirProgram, Pattern, Span, Step,
    TableSchema, Target, Type, ValuePattern, ValuePatternKind,
};
use crate::{
    builtins,
    diagnostic::{Category, Diagnostic},
    types::Prim,
    Error,
};
use crate::{core::operation::Operation, resolve::SymbolId};
use lexical::validate_lexical_ownership;
use operations::{
    algebra_signature, complex_arithmetic_signature, dense_signature, is_complex_arithmetic,
    is_new_dense_operation, layout_matches, mathesis_signature,
};
use std::collections::{HashMap, HashSet};

fn invalid(span: Span, detail: impl Into<String>) -> Error {
    Diagnostic::new(Category::InvalidHir, span, detail).into_error()
}

fn edge(id: ExprId, limit: usize, span: Span) -> Result<(), Error> {
    if id.0 >= limit {
        return Err(invalid(
            span,
            format!("invalid HIR expression reference {}", id.0),
        ));
    }
    Ok(())
}

fn introduce(symbols: &mut HashSet<SymbolId>, id: SymbolId, span: Span) -> Result<(), Error> {
    if !symbols.insert(id) {
        return Err(invalid(span, format!("duplicate HIR binding {}", id.0)));
    }
    Ok(())
}

fn definition(
    def: &Definition,
    limit: usize,
    symbols: &mut HashSet<SymbolId>,
    callables: &mut HashMap<SymbolId, usize>,
) -> Result<(), Error> {
    edge(def.body, limit, def.span)?;
    introduce(symbols, def.symbol, def.span)?;
    if !def.formals.is_empty() {
        callables.insert(def.symbol, def.formals.len());
    }
    for &formal in &def.formals {
        introduce(symbols, formal, def.span)?;
    }
    Ok(())
}

fn constructor(p: &HirProgram, id: SymbolId, span: Span) -> Result<&ConstructorDef, Error> {
    p.constructors
        .get(&id)
        .ok_or_else(|| invalid(span, format!("unknown HIR constructor {}", id.0)))
}

enum SymbolUse {
    Load,
    Reference,
    Call(usize),
}

fn check_arity(expected: usize, received: usize, span: Span) -> Result<(), Error> {
    if expected != received {
        return Err(invalid(
            span,
            format!("HIR target expects {expected} operand(s)"),
        ));
    }
    Ok(())
}

fn target(
    p: &HirProgram,
    target: &Target,
    arity: Option<usize>,
    span: Span,
    references: &mut Vec<(SymbolId, Span, SymbolUse)>,
) -> Result<(), Error> {
    let expected = match target {
        Target::Builtin(op) => {
            builtins::for_operation(*op)
                .ok_or_else(|| invalid(span, format!("unregistered HIR builtin {op:?}")))?
                .arity
        }
        Target::Projection(_) => 1,
        Target::Constructor(id) => constructor(p, *id, span)?.payload.len(),
        Target::Symbol(id) => {
            references.push((
                *id,
                span,
                arity.map_or(SymbolUse::Reference, SymbolUse::Call),
            ));
            return Ok(());
        }
    };
    if arity.is_none()
        && (expected == 0
            || matches!(
                target,
                Target::Builtin(
                    CoreOp::Apply | CoreOp::ApplyTuple | CoreOp::Execute | CoreOp::Default
                )
            ))
    {
        return Err(invalid(
            span,
            "invalid HIR reference to a special or nullary operation",
        ));
    }
    if let Some(received) = arity {
        check_arity(expected, received, span)?;
    }
    Ok(())
}

fn pattern_binders(
    p: &HirProgram,
    pattern: &ValuePattern,
    mut binder: impl FnMut(SymbolId, Span) -> Result<(), Error>,
) -> Result<(), Error> {
    use ValuePatternKind::*;
    let mut stack = vec![(pattern, 0)];
    while let Some((node, depth)) = stack.pop() {
        if depth >= 256 || node.span.start > node.span.end {
            return Err(invalid(
                Span::at(node.span.start),
                "invalid HIR pattern depth or span",
            ));
        }
        match &node.kind {
            Bind(symbol) => binder(*symbol, node.span)?,
            Constructor(id, fields) => {
                if constructor(p, *id, node.span)?.payload.len() != fields.len() {
                    return Err(invalid(node.span, "invalid HIR match constructor arity"));
                }
                stack.extend(fields.iter().rev().map(|n| (n, depth + 1)));
            }
            Tuple(parts) => stack.extend(parts.iter().rev().map(|n| (n, depth + 1))),
            Record(parts) => {
                let mut names = HashSet::new();
                for (name, field) in parts.iter().rev() {
                    if !names.insert(&**name) {
                        return Err(invalid(node.span, "duplicate HIR match record field"));
                    }
                    stack.push((field, depth + 1));
                }
            }
            Optional(Some(payload)) => stack.push((payload, depth + 1)),
            Optional(None) | Wildcard | Bool(_) | Unit => {}
        }
    }
    Ok(())
}

/// Validate the arena and witnesses before executing external or newly lowered
/// HIR. This checks structural integrity, not source typing or effect inference.
pub fn validate(p: &HirProgram) -> Result<(), Error> {
    if let Some(entry) = p.entry {
        edge(entry, p.arena.len(), Span::default())?;
    }
    let mut bindings = HashSet::new();
    let mut callables = HashMap::new();
    for (&id, def) in &p.constructors {
        if id != def.id {
            return Err(invalid(
                Span::default(),
                "HIR constructor identity disagrees with its key",
            ));
        }
        introduce(&mut bindings, id, Span::default())?;
    }
    for ((owner, name), id) in &p.standard_constructors {
        let def = constructor(p, *id, Span::default())?;
        if def.owner.name != *owner || &*def.name != name {
            return Err(invalid(
                Span::default(),
                "invalid standard HIR constructor identity",
            ));
        }
    }
    for d in p.groups.iter().flatten() {
        definition(d, p.arena.len(), &mut bindings, &mut callables)?;
    }
    let mut references = Vec::new();
    let mut source_ids = HashSet::new();
    let mut boundaries = HashSet::new();
    let mut escapes = Vec::new();
    for (i, node) in p.arena.iter().enumerate() {
        let at = node.span;
        if !source_ids.insert(node.source_id) {
            return Err(invalid(at, "duplicate HIR source occurrence identity"));
        }
        if at.start > at.end {
            return Err(invalid(Span::at(at.start), "inverted HIR source span"));
        }
        let child = |id| edge(id, i, at);
        let expected_lift = match &node.kind {
            ExprKind::Call(Target::Builtin(op), operands) => {
                if let Some(arity) = op.lift_arity() {
                    for &operand in operands {
                        child(operand)?;
                    }
                    check_arity(arity, operands.len(), at)?;
                    super::lift_for_call(*op, operands, &p.arena, &node.ty)
                        .map_err(|reason| invalid(at, reason))?
                } else {
                    None
                }
            }
            _ => None,
        };
        if node.lift_mode != expected_lift {
            return Err(invalid(
                at,
                "HIR lifting evidence disagrees with operand shape",
            ));
        }
        // No witness is required for an open/generic layout: callable
        // instantiation supplies it from the checked concrete result.
        if !layout_matches(p, &node.ty, node.table_schema.as_deref()) {
            return Err(invalid(at, "HIR table layout disagrees with checked type"));
        }
        if matches!(
            node.kind,
            ExprKind::Call(
                Target::Builtin(
                    CoreOp::Add
                        | CoreOp::Sub
                        | CoreOp::Mul
                        | CoreOp::Div
                        | CoreOp::Quotient
                        | CoreOp::Remainder
                        | CoreOp::Pow
                        | CoreOp::Negate
                        | CoreOp::Abs
                ),
                _
            )
        ) && !matches!(node.evidence, Some(Operation::Numeric { .. }))
        {
            return Err(invalid(at, "missing HIR arithmetic witness"));
        }
        if let Some(Operation::Numeric { operator, result }) = &node.evidence {
            let expected = builtins::lookup(operator).and_then(|op| op.op);
            if !matches!(&node.kind, ExprKind::Call(Target::Builtin(op), _) if Some(*op) == expected)
                || result != &node.ty
            {
                return Err(invalid(
                    at,
                    "HIR numeric witness disagrees with the checked call",
                ));
            }
        }
        match &node.kind {
            ExprKind::Literal(_) => {}
            ExprKind::Load(id) => references.push((*id, at, SymbolUse::Load)),
            ExprKind::Reference(t) => {
                target(p, t, None, at, &mut references)?;
                if let Target::Builtin(op) = t {
                    let spec = builtins::for_operation(*op).expect("validated target");
                    let mathesis = spec.math_signature().is_some();
                    let algebra = spec.algebra_signature().is_some();
                    let complex = is_complex_arithmetic(*op)
                        && node
                            .ty
                            .find_map(|t| matches!(t, Type::Prim(Prim::Cx)).then_some(()))
                            .is_some();
                    if is_new_dense_operation(*op) || mathesis || algebra || complex {
                        let mut output = &node.ty;
                        let mut inputs = Vec::with_capacity(spec.arity);
                        for _ in 0..spec.arity {
                            let Type::Fun(input, next) = output else {
                                return Err(invalid(
                                    at,
                                    if mathesis {
                                        "incomplete HIR Mathesis callable type"
                                    } else if algebra || complex {
                                        "incomplete HIR Algebra callable type"
                                    } else {
                                        "incomplete HIR dense-array callable type"
                                    },
                                ));
                            };
                            inputs.push(input.as_ref());
                            output = next;
                        }
                        if mathesis {
                            mathesis_signature(spec, &inputs, output, at)?;
                        } else if algebra {
                            algebra_signature(spec, &inputs, output, at)?;
                        } else if complex {
                            complex_arithmetic_signature([inputs[0], inputs[1]], output, at)?;
                        } else {
                            dense_signature(*op, &inputs, output, at)?;
                        }
                    }
                }
            }
            ExprKind::Call(t, ids) => {
                target(p, t, Some(ids.len()), at, &mut references)?;
                for &id in ids {
                    child(id)?;
                }
                if let Target::Builtin(op) = t {
                    let spec = builtins::for_operation(*op).expect("validated target");
                    let mathesis = spec.math_signature().is_some();
                    let algebra = spec.algebra_signature().is_some();
                    if is_complex_arithmetic(*op) {
                        let [left, right] = ids.as_slice() else {
                            return Err(invalid(at, "invalid HIR binary arithmetic arity"));
                        };
                        complex_arithmetic_signature(
                            [&p.arena[left.0].ty, &p.arena[right.0].ty],
                            &node.ty,
                            at,
                        )?;
                    }
                    if is_new_dense_operation(*op) || mathesis || algebra {
                        let inputs: Vec<_> = ids.iter().map(|id| &p.arena[id.0].ty).collect();
                        if mathesis {
                            mathesis_signature(spec, &inputs, &node.ty, at)?;
                        } else if algebra {
                            algebra_signature(spec, &inputs, &node.ty, at)?;
                        } else {
                            dense_signature(*op, &inputs, &node.ty, at)?;
                        }
                    }
                }
            }
            ExprKind::Tuple(ids) | ExprKind::Sequence(ids) => {
                for &id in ids {
                    child(id)?;
                }
            }
            ExprKind::Record(fields) => {
                let mut names = HashSet::new();
                for (name, id) in fields {
                    child(*id)?;
                    if !names.insert(&**name) {
                        return Err(invalid(at, "duplicate HIR record field"));
                    }
                }
            }
            ExprKind::Cases {
                branches,
                otherwise,
            } => {
                for &(condition, result) in branches {
                    child(condition)?;
                    child(result)?;
                }
                child(*otherwise)?;
            }
            ExprKind::Closure { formals, body } => {
                child(*body)?;
                for &id in formals {
                    introduce(&mut bindings, id, at)?;
                }
            }
            ExprKind::Pipeline(stages) => {
                if stages.is_empty() {
                    return Err(invalid(at, "empty HIR pipeline"));
                }
                for (j, stage) in stages.iter().enumerate() {
                    child(stage.expr)?;
                    match (j, stage.input) {
                        (0, None) => {}
                        (_, Some(id)) if j != 0 => introduce(&mut bindings, id, at)?,
                        _ => return Err(invalid(at, "invalid HIR pipeline input binding")),
                    }
                }
            }
            ExprKind::Fallible {
                boundary,
                steps,
                tail,
            } => {
                if !boundaries.insert(*boundary) {
                    return Err(invalid(at, "duplicate HIR error boundary"));
                }
                validate_steps(steps, *tail, i, at, &mut bindings, &mut callables)?;
            }
            ExprKind::Block { steps, tail, .. } => {
                validate_steps(steps, *tail, i, at, &mut bindings, &mut callables)?;
            }
            ExprKind::Raise { boundary, payload } => {
                child(*payload)?;
                escapes.push((*boundary, at));
            }
            ExprKind::Propagate { boundary, result } => {
                child(*result)?;
                escapes.push((*boundary, at));
            }
            ExprKind::Match { value, arms } => {
                child(*value)?;
                if arms.is_empty() {
                    return Err(invalid(at, "HIR match has no arms"));
                }
                for arm in arms {
                    child(arm.body)?;
                    pattern_binders(p, &arm.pattern, |id, span| {
                        introduce(&mut bindings, id, span)
                    })?;
                }
            }
            ExprKind::Recover { result, arms } => {
                child(*result)?;
                for arm in arms {
                    child(arm.body)?;
                    match &arm.pattern {
                        Pattern::Wildcard => {}
                        Pattern::Bind(id) => introduce(&mut bindings, *id, arm.span)?,
                        Pattern::Constructor { id, fields } => {
                            let expected = constructor(p, *id, arm.span)?.payload.len();
                            if fields.len() != expected {
                                return Err(invalid(
                                    arm.span,
                                    "invalid HIR recovery pattern arity",
                                ));
                            }
                            for &binding in fields.iter().flatten() {
                                introduce(&mut bindings, binding, arm.span)?;
                            }
                        }
                    }
                }
            }
        }
    }
    for (id, at, usage) in references {
        if !bindings.contains(&id) {
            return Err(invalid(at, format!("unknown HIR symbol {}", id.0)));
        }
        let given = match usage {
            SymbolUse::Load => continue,
            SymbolUse::Reference => None,
            SymbolUse::Call(n) => Some(n),
        };
        let expected = callables
            .get(&id)
            .ok_or_else(|| invalid(at, "HIR callable target refers to a value binding"))?;
        if given.is_some_and(|n| n != *expected) {
            return Err(invalid(
                at,
                format!("HIR callable target expects {expected} operand(s)"),
            ));
        }
    }
    for (id, at) in escapes {
        if !boundaries.contains(&id) {
            return Err(invalid(at, format!("unknown HIR error boundary {}", id.0)));
        }
    }
    validate_lexical_ownership(p)
}

/// Lowering owns a contiguous, post-order tree, not a shared expression DAG.
/// This gives an exact scope test without walking ancestor chains per load.
fn validate_steps(
    steps: &[Step],
    tail: Option<ExprId>,
    limit: usize,
    span: Span,
    bindings: &mut HashSet<SymbolId>,
    callables: &mut HashMap<SymbolId, usize>,
) -> Result<(), Error> {
    for step in steps {
        match step {
            Step::Define(def) => definition(def, limit, bindings, callables)?,
            Step::Evaluate { binding, body } => {
                edge(*body, limit, span)?;
                if let Some(id) = binding {
                    introduce(bindings, *id, span)?;
                }
            }
        }
    }
    if let Some(tail) = tail {
        edge(tail, limit, span)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hir::{lower, LiftMode};

    fn checked_hir(source: &str) -> HirProgram {
        let program = crate::parser::parse(source).unwrap();
        let names = crate::resolve::resolve(&program).unwrap();
        let types = crate::infer::infer(&program, &names).unwrap();
        lower(&program, &types, &names).unwrap()
    }

    #[test]
    fn dense_pointwise_plans_are_checked_before_execution() {
        let ir =
            checked_hir("a := form [1n] [1z]; b := + a a; c := + a 1z; d := + 1z a; e := -: a;");
        validate(&ir).unwrap();
        let modes = ir
            .groups
            .iter()
            .flatten()
            .filter_map(|d| ir.node(d.body).lift_mode)
            .collect::<Vec<_>>();
        assert_eq!(
            modes,
            [
                LiftMode::ArrayPair,
                LiftMode::LeftArray,
                LiftMode::RightArray,
                LiftMode::LeftArray
            ]
        );
        let mut forged = ir.clone();
        let i = forged
            .arena
            .iter()
            .position(|node| node.lift_mode == Some(LiftMode::ArrayPair))
            .unwrap();
        forged.arena[i].lift_mode = Some(LiftMode::Pairwise);
        assert!(validate(&forged)
            .unwrap_err()
            .message
            .contains("lifting evidence"));
        forged.arena[i].lift_mode = Some(LiftMode::ArrayPair);
        forged.arena[i].ty = Type::Prim(Prim::Z);
        assert!(validate(&forged)
            .unwrap_err()
            .message
            .contains("result shape"));
    }

    #[test]
    fn a_forged_array_callable_signature_is_not_admitted() {
        let mut ir = checked_hir("f:[n]->[z]->array[z] := &form; g:array[z]->[n] := &dim;");
        validate(&ir).unwrap();
        let node = ir
            .arena
            .iter_mut()
            .find(|node| {
                matches!(
                    node.kind,
                    ExprKind::Reference(Target::Builtin(CoreOp::ArrayForm))
                )
            })
            .unwrap();
        node.ty = Type::Fun(Box::new(Type::Prim(Prim::N)), Box::new(Type::Prim(Prim::Z)));
        assert!(validate(&ir)
            .unwrap_err()
            .message
            .contains("incomplete HIR dense-array callable type"));
    }
    #[test]
    fn forged_algebra_result_and_reference_domains_are_rejected() {
        let ir = checked_hir(
            r#"<<"lin"; m:array[n] := form [1 1] [2];
                det:z^linerr := lin.det m; f:array[n]->z^linerr := &lin.det;"#,
        );
        validate(&ir).unwrap();
        for reference in [false, true] {
            let mut forged = ir.clone();
            let node = forged
                .arena
                .iter_mut()
                .find(|node| match node.kind {
                    ExprKind::Reference(Target::Builtin(CoreOp::LinDet)) => reference,
                    ExprKind::Call(Target::Builtin(CoreOp::LinDet), _) => !reference,
                    _ => false,
                })
                .unwrap();
            let value = match &mut node.ty {
                Type::Fun(_, out) => out.as_mut(),
                result => result,
            };
            let Type::Result(ok, _) = value else {
                panic!("expected a checked fallible determinant")
            };
            **ok = Type::Prim(Prim::Q);
            let diagnostic = validate(&forged).unwrap_err();
            assert!(
                diagnostic.message.contains("HIR lin.det signature"),
                "{diagnostic}"
            );
        }
    }
    #[test]
    fn forged_complex_arithmetic_and_projections_are_rejected() {
        let base = checked_hir(
            r#"<<"cx"; a := cx.make 1 2; b := + a a;
                f:cx->cx->cx := &/; re:cx->r := &cx.re;"#,
        );
        validate(&base).unwrap();
        for variant in [CoreOp::Add, CoreOp::Div, CoreOp::CxRe] {
            let mut forged = base.clone();
            let node = forged
                .arena
                .iter_mut()
                .find(|node| match node.kind {
                    ExprKind::Reference(Target::Builtin(op))
                    | ExprKind::Call(Target::Builtin(op), _) => op == variant,
                    _ => false,
                })
                .unwrap();
            let real = Type::Prim(Prim::R);
            match (&mut node.kind, &mut node.ty) {
                (ExprKind::Call(_, _), ty) => {
                    *ty = real.clone();
                    let Some(Operation::Numeric { result, .. }) = &mut node.evidence else {
                        panic!("expected arithmetic witness")
                    };
                    *result = real;
                }
                (ExprKind::Reference(_), Type::Fun(first, _)) => **first = real,
                _ => panic!("expected a checked complex operation"),
            }
            let diagnostic = validate(&forged).unwrap_err();
            assert!(
                diagnostic.message.contains("HIR complex arithmetic")
                    || diagnostic.message.contains("HIR cx.re signature"),
                "{diagnostic}"
            );
        }
    }
}
