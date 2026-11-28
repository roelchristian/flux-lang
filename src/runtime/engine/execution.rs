//! Strict expression, block, pattern and suspended-action execution.
use super::{
    array_builder,
    control::{succeeded, Control, Failure},
    evidence::{contains_open_row, dynamic_schema_operation, result_type},
    primitives::witnessed_op,
    Eval,
};
use crate::{
    hir::{CoreOp, ExprId, ExprKind, LiftMode, Literal, Pattern, Span, Step, Target},
    hir_eval::{
        as_bool, decimal, domain, effects, error, finite, function, invariant, native,
        numerical_plan, parse_exact, projection, resources, wide, Action, Domain, Exact, Function,
        R, V,
    },
    resolve::SymbolId,
    types::{Prim, Type},
};
use std::rc::Rc;

impl Eval<'_> {
    pub(in crate::hir_eval) fn eval(&mut self, id: ExprId, env: usize) -> R<V> {
        let p = self.p;
        let node = p.node(id);
        let r = self.with_expression(node.span, |eval| eval.eval_kind(id, env));
        r.and_then(|v| {
            let checked = if matches!(node.kind, ExprKind::Call(Target::Builtin(_), _)) {
                wide::guard_embedded(v)
            } else {
                wide::guard_value(v)
            };
            checked.map_err(|message| error(node.span, message))
        })
        .map(|v| self.specialize_fun(v, &node.ty, env))
    }
    fn eval_kind(&mut self, id: ExprId, env: usize) -> R<V> {
        let p = self.p;
        let n = p.node(id);
        let s = n.span;
        let convert = |message| error(s, message);
        Ok(match &n.kind {
            ExprKind::Literal(v) => match v {
                Literal::Integer(i) => {
                    let ty = self.type_at(&n.ty, env);
                    if ty == Type::Prim(Prim::R) {
                        finite(*i as f64).map_err(convert)?
                    } else {
                        let d = domain(&ty).ok_or_else(|| {
                            error(s, "integer literal has no checked numeric domain")
                        })?;
                        if d == Domain::Decimal {
                            decimal::decimal_literal(&i.to_string()).map_err(convert)?
                        } else {
                            V::Exact(Exact::integer(*i, d).as_domain(d).map_err(convert)?)
                        }
                    }
                }
                Literal::Numeric(text) => {
                    let ty = self.type_at(&n.ty, env);
                    if text.ends_with('r') || ty == Type::Prim(Prim::R) {
                        let raw = text.strip_suffix('r').unwrap_or(text);
                        finite(raw.parse().map_err(|_| error(s, "invalid real literal"))?)
                            .map_err(convert)?
                    } else {
                        parse_exact(text, &ty).map_err(convert)?
                    }
                }
                Literal::Bool(b) => V::Bool(*b),
                Literal::String(text) => V::String(Rc::from(text.as_ref())),
                Literal::Char(c) => V::Char(*c),
                Literal::Unit => V::Unit,
            },
            ExprKind::Load(sid) => self.load(env, *sid, s)?,
            ExprKind::Reference(Target::Symbol(sid)) => self.load(env, *sid, s)?,
            ExprKind::Reference(Target::Constructor(id)) => {
                let arity = p
                    .constructors
                    .get(id)
                    .ok_or_else(|| invariant(s, "checked constructor is absent"))?
                    .payload
                    .len();
                if arity == 0 {
                    self.make_variant(*id, Vec::new(), s)?
                } else {
                    function(Function::Constructor(*id, arity))
                }
            }
            ExprKind::Reference(Target::Projection(field)) => {
                function(Function::Projection(field.clone()))
            }
            ExprKind::Reference(Target::Builtin(op)) => function(Function::Core(
                *op,
                self.type_at(&n.ty, env),
                n.table_schema.clone(),
            )),
            ExprKind::Call(Target::Builtin(CoreOp::TableFrom), args)
                if n.table_schema.is_none()
                    && matches!(
                        &p.node(args[0]).kind,
                        ExprKind::Call(Target::Builtin(CoreOp::Rows), _)
                    ) =>
            {
                // The direct round trip preserves its actual input layout.
                // Two arguments can share an open row type while differing
                // in column order; the other argument is not its witness.
                let ExprKind::Call(_, source) = &p.node(args[0]).kind else {
                    unreachable!("checked row round trip")
                };
                match self.eval(source[0], env)? {
                    table @ V::Table(_, _) => table,
                    _ => return Err(invariant(s, "checked row round trip needs a table")),
                }
            }
            ExprKind::Call(Target::Builtin(CoreOp::Default), args) => {
                let [fallback, stored] = args.as_slice() else {
                    return Err(error(s, "invalid fallback operands"));
                };
                match self.eval(*stored, env)? {
                    V::Optional(Some(v)) => v.as_ref().clone(),
                    V::Optional(None) => self.eval(*fallback, env)?,
                    _ => return Err(error(s, "fallback requires an optional second operand")),
                }
            }
            ExprKind::Call(target, ids) => {
                if matches!(target, Target::Builtin(CoreOp::ArrayForm)) {
                    if let Some(result) = array_builder::try_form_range(self, id, env, ids) {
                        return result;
                    }
                }
                if n.lift_mode == Some(LiftMode::LeftArray)
                    && matches!(target, Target::Builtin(op) if numerical_plan::can_start(*op))
                {
                    if let Some(result) = numerical_plan::try_eval(self, id, env) {
                        return result;
                    }
                }
                let f = match target {
                    Target::Symbol(sid) => Some(self.load(env, *sid, s)?),
                    Target::Builtin(_) | Target::Projection(_) | Target::Constructor(_) => None,
                };
                let args = ids
                    .iter()
                    .map(|id| self.eval(*id, env))
                    .collect::<R<Vec<_>>>()?;
                let arg_types = |eval: &Self| {
                    ids.iter()
                        .map(|id| eval.type_at(&p.node(*id).ty, env))
                        .collect::<Vec<_>>()
                };
                match target {
                    Target::Builtin(CoreOp::Apply | CoreOp::ApplyTuple) => {
                        let [f, arg]: [V; 2] = args
                            .try_into()
                            .map_err(|_| error(s, "invalid typed application"))?;
                        let input = self.type_at(&p.node(ids[1]).ty, env);
                        let output = self.type_at(&n.ty, env);
                        if matches!(target, Target::Builtin(CoreOp::ApplyTuple)) {
                            let (V::Tuple(v), Type::Tuple(types)) = (arg, input) else {
                                return Err(error(s, "@@ expects a checked tuple bundle"));
                            };
                            let mut f = f;
                            let sig = self.type_at(&p.node(ids[0]).ty, env);
                            for (i, (operand, actual)) in v.iter().zip(types).enumerate() {
                                let next = result_type(&sig, i + 1);
                                let result = if i + 1 == v.len() {
                                    output.clone()
                                } else {
                                    next.clone()
                                };
                                f = self.apply_unary(f, operand.clone(), actual, result, s)?;
                            }
                            f
                        } else {
                            self.apply_unary(f, arg, input, output, s)?
                        }
                    }
                    Target::Builtin(op) => {
                        let ty = match &n.evidence {
                            Some(crate::core::operation::Operation::Numeric { result, .. }) => {
                                result
                            }
                            _ => &n.ty,
                        };
                        let ty = self.type_at(ty, env);
                        if let Some(op) = witnessed_op(*op) {
                            self.witnessed_core(
                                op,
                                args,
                                &arg_types(self),
                                &ty,
                                n.table_schema.as_ref(),
                                s,
                            )?
                        } else {
                            let layout = if dynamic_schema_operation(*op)
                                && self.frames[env].type_scope.is_some()
                            {
                                self.p.instance_schema(&ty)
                            } else {
                                None
                            };
                            self.core(
                                *op,
                                args,
                                &ty,
                                layout.as_ref().or(n.table_schema.as_ref()),
                                n.lift_mode,
                                s,
                            )?
                        }
                    }
                    Target::Constructor(id) => self.make_variant(*id, args, s)?,
                    Target::Projection(field) => {
                        let [v]: [V; 1] = args
                            .try_into()
                            .map_err(|_| error(s, "invalid projection operand count"))?;
                        projection(field, v, s)?
                    }
                    Target::Symbol(_) => {
                        let mut f = f.expect("callee loaded");
                        if let Some(sig) = n.callee_type.as_ref() {
                            f = self.specialize_fun(f, sig, env);
                            if !matches!(&f, V::Fun(f) if matches!(f.code, Function::User { open_rows: true, .. }))
                            {
                                return self.apply(f, args, s);
                            }
                        }
                        let input = arg_types(self);
                        let output = self.type_at(&n.ty, env);
                        self.apply_checked(f, args, Some(input), Some(output), s)?
                    }
                }
            }
            ExprKind::Tuple(ids) | ExprKind::Sequence(ids) => {
                if matches!(n.kind, ExprKind::Sequence(_))
                    && ids.len() > resources::limits().max_elements
                {
                    return Err(error(s, "sequence exceeds the collection limit"));
                }
                let xs = ids
                    .iter()
                    .map(|id| self.eval(*id, env))
                    .collect::<R<Vec<_>>>()?
                    .into();
                if matches!(n.kind, ExprKind::Tuple(_)) {
                    V::Tuple(xs)
                } else {
                    V::Sequence(xs)
                }
            }
            ExprKind::Record(fields) => V::Record(
                fields
                    .iter()
                    .map(|(k, v)| Ok((k.clone(), self.eval(*v, env)?)))
                    .collect::<R<Vec<_>>>()?
                    .into(),
            ),
            ExprKind::Cases {
                branches,
                otherwise,
            } => {
                for &(a, b) in branches {
                    if as_bool(&self.eval(a, env)?).map_err(convert)? {
                        return self.eval(b, env);
                    }
                }
                self.eval(*otherwise, env)?
            }
            ExprKind::Closure { formals, body } => function(Function::User {
                formals: formals.clone().into(),
                body: *body,
                env,
                declared: n.ty.clone(),
                open_rows: contains_open_row(&n.ty),
            }),
            ExprKind::Pipeline(stages) => {
                let mut value = V::Unit;
                for stage in stages {
                    let scope = if let Some(input) = stage.input {
                        let scope = self.frame(env);
                        self.put(scope, input, value);
                        scope
                    } else {
                        env
                    };
                    value = self.eval(stage.expr, scope)?;
                }
                value
            }
            ExprKind::Fallible {
                boundary,
                steps,
                tail,
            } => match self.block(steps, *tail, env) {
                Ok(v) => succeeded(v),
                Err(Control::Raise {
                    boundary: target,
                    failure,
                }) if target == *boundary => V::Outcome(Err(failure)),
                Err(other) => return Err(other),
            },
            ExprKind::Raise { boundary, payload } => {
                let v = self.eval(*payload, env)?;
                return Err(Control::Raise {
                    boundary: *boundary,
                    failure: Failure::new(Rc::new(v), s, None),
                });
            }
            ExprKind::Propagate { boundary, result } => match self.eval(*result, env)? {
                V::Outcome(Ok(v)) => v.as_ref().clone(),
                V::Outcome(Err(mut f)) => {
                    Rc::make_mut(&mut f).propagations.push(s);
                    return Err(Control::Raise {
                        boundary: *boundary,
                        failure: f,
                    });
                }
                _ => return Err(invariant(s, "checked propagation operand was not Result")),
            },
            ExprKind::Match { value, arms } => {
                let value = self.eval(*value, env)?;
                for arm in arms {
                    if let Some(bindings) = self.match_value(&arm.pattern, &value)? {
                        let local = self.frame(env);
                        for (id, value) in bindings {
                            self.put(local, id, value.clone());
                        }
                        return self.eval(arm.body, local);
                    }
                }
                return Err(invariant(s, "checked match did not cover the scrutinee"));
            }
            ExprKind::Recover { result, arms } => match self.eval(*result, env)? {
                V::Outcome(Ok(v)) => v.as_ref().clone(),
                V::Outcome(Err(f)) => {
                    for arm in arms {
                        let bindings = match &arm.pattern {
                            Pattern::Wildcard => Vec::new(),
                            Pattern::Bind(id) => vec![(*id, f.payload.as_ref().clone())],
                            Pattern::Constructor { id, fields } => {
                                let V::Variant(variant) = f.payload.as_ref() else {
                                    continue;
                                };
                                let payload = &variant.fields;
                                if variant.constructor != *id {
                                    continue;
                                }
                                if fields.len() != payload.len() {
                                    return Err(invariant(
                                        arm.span,
                                        "checked recovery pattern arity differs",
                                    ));
                                }
                                fields
                                    .iter()
                                    .zip(payload.iter())
                                    .filter_map(|(id, v)| id.map(|i| (i, v.clone())))
                                    .collect()
                            }
                        };
                        let local = self.frame(env);
                        for (id, v) in bindings {
                            self.put(local, id, v);
                        }
                        return self.eval(arm.body, local);
                    }
                    return Err(invariant(
                        s,
                        "checked recovery did not cover the failure variant",
                    ));
                }
                _ => return Err(invariant(s, "checked recovery operand was not Result")),
            },
            ExprKind::Block {
                suspended: true, ..
            } => V::Action(Rc::new(Action::Block { expr: id, env })),
            ExprKind::Block { steps, tail, .. } => self.block(steps, *tail, env)?,
        })
    }
    fn match_value<'v>(
        &mut self,
        pattern: &crate::hir::ValuePattern,
        value: &'v V,
    ) -> R<Option<Vec<(SymbolId, &'v V)>>> {
        use crate::hir::ValuePatternKind as P;
        let mut bindings = Vec::new();
        let mut stack = vec![(pattern, value)];
        while let Some((pattern, value)) = stack.pop() {
            self.tick(pattern.span)?;
            match (&pattern.kind, value) {
                (P::Wildcard, _) => {}
                (P::Bind(id), value) => bindings.push((*id, value)),
                (P::Constructor(id, fields), V::Variant(actual))
                    if *id == actual.constructor && fields.len() == actual.fields.len() =>
                {
                    stack.extend(fields.iter().zip(actual.fields.iter()).rev());
                }
                (P::Tuple(parts), V::Tuple(values)) if parts.len() == values.len() => {
                    stack.extend(parts.iter().zip(values.iter()).rev());
                }
                (P::Record(parts), V::Record(values)) => {
                    for (name, part) in parts.iter().rev() {
                        let Some((_, value)) = values.iter().find(|(key, _)| key == name) else {
                            return Ok(None);
                        };
                        stack.push((part, value));
                    }
                }
                (P::Optional(Some(part)), V::Optional(Some(value))) => stack.push((part, value)),
                (P::Optional(None), V::Optional(None)) | (P::Unit, V::Unit) => {}
                (P::Bool(expected), V::Bool(actual)) if expected == actual => {}
                _ => return Ok(None),
            }
        }
        Ok(Some(bindings))
    }
    pub(in crate::hir_eval) fn block(
        &mut self,
        steps: &[Step],
        tail: Option<ExprId>,
        parent: usize,
    ) -> R<V> {
        let env = self.frame(parent);
        for step in steps {
            if let Step::Define(d) = step {
                if !d.formals.is_empty() {
                    self.define_fun(d, env)
                }
            }
        }
        for step in steps {
            match step {
                Step::Define(d) if d.formals.is_empty() => {
                    let v = self.eval(d.body, env)?;
                    self.put(env, d.symbol, v);
                }
                Step::Evaluate { binding, body } => {
                    let v = self.eval(*body, env)?;
                    if let Some(s) = binding {
                        self.put(env, *s, v)
                    }
                }
                _ => {}
            }
        }
        tail.map(|id| self.eval(id, env)).unwrap_or(Ok(V::Unit))
    }
}

impl Eval<'_> {
    pub(in crate::hir_eval) fn execute(&mut self, v: V, span: Span) -> R<V> {
        let V::Action(action) = v else {
            return Err(error(span, "expected a suspended action"));
        };
        if let Some(result) = effects::apply_action(&action, self, span) {
            return result;
        }
        if let Some(result) = native::apply_action(&action, self, span) {
            return result;
        }
        match action.as_ref() {
            Action::Library(..) => Err(invariant(span, "unknown standard library action")),
            Action::IoRead(_) | Action::IoWrite(..) | Action::IoExists(_) | Action::IoPrint(_) => {
                Err(invariant(span, "fallible I/O action has no implementation"))
            }
            Action::CsvRead(..) | Action::CsvWrite(..) => {
                Err(invariant(span, "fallible CSV action has no implementation"))
            }
            Action::Block { expr, env } => {
                let p = self.p;
                let ExprKind::Block { steps, tail, .. } = &p.node(*expr).kind else {
                    return Err(error(span, "invalid action expression in typed HIR"));
                };
                if self.effects >= 128 {
                    return Err(error(
                        span,
                        "prototype action recursion limit (128) exceeded",
                    ));
                }
                self.effects += 1;
                let result = self.block(steps, *tail, *env);
                self.effects -= 1;
                result
            }
        }
    }
}
