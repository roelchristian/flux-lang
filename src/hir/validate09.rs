//! Structural admission checks for stand-alone typed HIR.
use super::*;
use crate::{
    builtins,
    diagnostic::{Category, Diagnostic},
    Error,
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
    if arity.is_some_and(|n| n != expected) {
        return Err(invalid(
            span,
            format!("HIR target expects {expected} operand(s)"),
        ));
    }
    Ok(())
}

fn layout_matches(p: &HirProgram, mut ty: &Type, witness: Option<&TableSchema>) -> bool {
    while let Type::Action(out) | Type::Result(out, _) | Type::Fun(_, out) = ty {
        ty = out;
    }
    if let Type::Table(row) = ty {
        match row.as_ref() {
            Type::Record(row) if row.tail.is_none() => {
                return matches!(witness, Some(TableSchema::Named(fields)) if fields == &row.fields);
            }
            Type::Tuple(row) => {
                return matches!(witness, Some(TableSchema::Positional(fields)) if fields == row);
            }
            Type::Nominal(id) => return p.nominal_schemas.get(id).map(Rc::as_ref) == witness,
            _ => {}
        }
    }
    witness.is_none()
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
            ExprKind::Call(Target::Builtin(op), operands) if op.lift_arity().is_some() => {
                for &operand in operands {
                    child(operand)?;
                }
                let seq = |i: usize| {
                    operands
                        .get(i)
                        .is_some_and(|id| matches!(p.arena[id.0].ty, Type::Seq(_)))
                };
                Some(LiftMode::from_shapes(seq(0), seq(1)))
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
            ExprKind::Reference(t) => target(p, t, None, at, &mut references)?,
            ExprKind::Call(t, ids) => {
                target(p, t, Some(ids.len()), at, &mut references)?;
                for &id in ids {
                    child(id)?;
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
fn validate_lexical_ownership(p: &HirProgram) -> Result<(), Error> {
    let mut begins = Vec::with_capacity(p.arena.len());
    let mut owned = vec![false; p.arena.len()];
    let mut parents = vec![None; p.arena.len()];
    let mut barriers = vec![false; p.arena.len()];
    let mut local = HashMap::<SymbolId, (usize, usize)>::new();
    let mut uses = Vec::new();
    for (i, node) in p.arena.iter().enumerate() {
        let mut first = None;
        let mut last = None;
        let mut child = |id: ExprId| {
            edge(id, i, node.span)?;
            if std::mem::replace(&mut owned[id.0], true)
                || last.is_some_and(|previous| previous + 1 != begins[id.0])
            {
                return Err(invalid(
                    node.span,
                    "noncontiguous or multiply owned HIR subtree",
                ));
            }
            parents[id.0] = Some(i);
            first.get_or_insert(begins[id.0]);
            last = Some(id.0);
            Ok(())
        };
        let mut scope = |symbol, id: ExprId| {
            local.insert(symbol, (begins[id.0], id.0));
        };
        match &node.kind {
            ExprKind::Load(sym) | ExprKind::Reference(Target::Symbol(sym)) => uses.push((*sym, i)),
            ExprKind::Call(target, args) => {
                if let Target::Symbol(sym) = target {
                    uses.push((*sym, i));
                }
                for &id in args {
                    child(id)?;
                }
            }
            ExprKind::Tuple(ids) | ExprKind::Sequence(ids) => {
                for &id in ids {
                    child(id)?;
                }
            }
            ExprKind::Record(fields) => {
                for (_, id) in fields {
                    child(*id)?;
                }
            }
            ExprKind::Cases {
                branches,
                otherwise,
            } => {
                for &(c, a) in branches {
                    child(c)?;
                    child(a)?;
                }
                child(*otherwise)?;
            }
            ExprKind::Closure { formals, body } => {
                child(*body)?;
                barriers[body.0] = true;
                for &symbol in formals {
                    scope(symbol, *body);
                }
            }
            ExprKind::Pipeline(stages) => {
                for stage in stages {
                    child(stage.expr)?;
                    if let Some(symbol) = stage.input {
                        scope(symbol, stage.expr);
                    }
                }
            }
            ExprKind::Block { steps, tail, .. } | ExprKind::Fallible { steps, tail, .. } => {
                let mut slots = Vec::new();
                let suspended = matches!(
                    node.kind,
                    ExprKind::Block {
                        suspended: true,
                        ..
                    }
                );
                for step in steps {
                    match step {
                        Step::Define(d) => {
                            child(d.body)?;
                            barriers[d.body.0] = suspended || !d.formals.is_empty();
                            slots.push(d.symbol);
                            for &formal in &d.formals {
                                scope(formal, d.body);
                            }
                        }
                        Step::Evaluate { binding, body } => {
                            child(*body)?;
                            barriers[body.0] = suspended;
                            slots.extend(*binding);
                        }
                    }
                }
                if let Some(id) = tail {
                    child(*id)?;
                    barriers[id.0] = suspended;
                }
                for slot in slots {
                    local.insert(slot, (first.unwrap_or(i), i));
                }
            }
            ExprKind::Raise { payload, .. } => child(*payload)?,
            ExprKind::Propagate { result, .. } => child(*result)?,
            ExprKind::Match { value, arms } => {
                child(*value)?;
                for arm in arms {
                    child(arm.body)?;
                    pattern_binders(p, &arm.pattern, |id, _| {
                        scope(id, arm.body);
                        Ok(())
                    })?;
                }
            }
            ExprKind::Recover { result, arms } => {
                child(*result)?;
                for arm in arms {
                    child(arm.body)?;
                    match &arm.pattern {
                        Pattern::Wildcard => {}
                        Pattern::Bind(symbol) => scope(*symbol, arm.body),
                        Pattern::Constructor { fields, .. } => {
                            for &symbol in fields.iter().flatten() {
                                scope(symbol, arm.body);
                            }
                        }
                    }
                }
            }
            ExprKind::Literal(_) | ExprKind::Reference(_) => {}
        }
        if last.is_some_and(|end| end + 1 != i) {
            return Err(invalid(node.span, "noncontiguous HIR subtree"));
        }
        begins.push(first.unwrap_or(i));
    }
    let mut root = |id: ExprId, span: Span| {
        if std::mem::replace(&mut owned[id.0], true) {
            return Err(invalid(span, "multiply owned HIR root"));
        }
        Ok(())
    };
    for d in p.groups.iter().flatten() {
        root(d.body, d.span)?;
        for &formal in &d.formals {
            local.insert(formal, (begins[d.body.0], d.body.0));
        }
    }
    if let Some(entry) = p.entry {
        root(entry, Span::default())?;
    }
    if owned.contains(&false) {
        return Err(invalid(Span::default(), "unowned HIR expression"));
    }
    for (symbol, i) in uses {
        if let Some(&(first, last)) = local.get(&symbol) {
            if !(first..=last).contains(&i) {
                return Err(invalid(
                    p.arena[i].span,
                    "HIR reference outside its lexical owner",
                ));
            }
        }
    }
    // Parents are higher in a post-order arena. Inherit the nearest
    // boundary unless this edge crosses a closure or suspended action.
    let mut active = vec![None; p.arena.len()];
    for (i, node) in p.arena.iter().enumerate().rev() {
        let nearest = if barriers[i] {
            None
        } else {
            parents[i].and_then(|p| active[p])
        };
        active[i] = match node.kind {
            ExprKind::Fallible { boundary, .. } => Some(boundary),
            ExprKind::Raise { boundary, .. } | ExprKind::Propagate { boundary, .. } => {
                if nearest != Some(boundary) {
                    return Err(invalid(
                        node.span,
                        "HIR escape does not target its nearest lexical error boundary",
                    ));
                }
                nearest
            }
            _ => nearest,
        };
    }
    Ok(())
}

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
