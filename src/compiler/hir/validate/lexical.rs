//! Lexical ownership of checked HIR definitions and references.
use super::{
    edge, invalid, pattern_binders, Error, ExprId, ExprKind, HashMap, HirProgram, Pattern, Span,
    Step, SymbolId, Target,
};

pub(super) fn validate_lexical_ownership(p: &HirProgram) -> Result<(), Error> {
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
